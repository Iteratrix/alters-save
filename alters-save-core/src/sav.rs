//! Outer file framing: plaintext prefix + UE `SerializeCompressed` zlib chunks.
//!
//! Layout of a world save (`*-ACT<n>-day-<d>-v-6.sav`):
//!
//! ```text
//! [plaintext prefix: header, ElbSaveMeta, load-screen details, GUID lists]
//! [chunk]*                          <- until EOF
//! ```
//!
//! The prefix contains exactly two little-endian `i32` fields whose value
//! equals `file_len - offset - 4` (they measure "bytes from here to EOF"):
//! the inner-archive size and the compressed-payload size. Both must be
//! rewritten whenever the compressed payload changes length; the game's
//! save-scan thread crashes with an access violation if the first one
//! overshoots the file.
//!
//! Each chunk is:
//!
//! ```text
//! [u64 tag 0x22222222_9E2A83C1] [u64 chunk_size 0x20000] [u8 format (3 = zlib)]
//! [u64 total_compressed] [u64 total_uncompressed]
//! [u64 compressed] [u64 uncompressed]
//! [compressed bytes]
//! ```
//!
//! Every chunk carries exactly one 128 KiB block (the last one smaller), so
//! the total and per-chunk size pairs are always equal.

use std::io::Read as _;

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::error::{Error, Result};

const CHUNK_MAGIC: u32 = 0x9E2A_83C1;
const CHUNK_TAG: u64 = 0x2222_2222_9E2A_83C1;
const CHUNK_BLOCK: usize = 0x2_0000;
const CHUNK_HEADER_LEN: usize = 8 + 8 + 1 + 32;
const MIN_SIZE_FIELD: i64 = 1000;

/// Compression format byte from a chunk header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionFormat {
    Zlib,
    Unknown(u8),
}

/// Inner-archive serialization version, read from the version int that
/// follows the inner-archive size field.
///
/// Observed values: `2` (game builds around late 2025) and `3` (current).
/// The tagged-property header layout differs between them, so body-editing
/// code must branch on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveVersion {
    V2,
    V3,
    Other(i32),
}

impl From<i32> for ArchiveVersion {
    fn from(value: i32) -> Self {
        match value {
            2 => Self::V2,
            3 => Self::V3,
            other => Self::Other(other),
        }
    }
}

impl From<u8> for CompressionFormat {
    fn from(value: u8) -> Self {
        match value {
            3 => Self::Zlib,
            other => Self::Unknown(other),
        }
    }
}

/// A parsed save file: plaintext prefix plus the decompressed body.
///
/// The body is the concatenation of all inflated chunks - a stream of the
/// game's custom "Elb" object records. Edit it (in place or with length
/// changes), then call [`SaveFile::to_bytes`] to rebuild a consistent file.
#[derive(Debug, Clone)]
pub struct SaveFile {
    prefix: Vec<u8>,
    /// Decompressed body; freely editable.
    pub body: Vec<u8>,
    eof_relative_fields: [usize; 2],
    archive_version: ArchiveVersion,
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let end = offset.checked_add(8)?;
    let slice = bytes.get(offset..end)?;
    let array: [u8; 8] = slice.try_into().ok()?;
    Some(u64::from_le_bytes(array))
}

fn read_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    let end = offset.checked_add(4)?;
    let slice = bytes.get(offset..end)?;
    let array: [u8; 4] = slice.try_into().ok()?;
    Some(i32::from_le_bytes(array))
}

struct ChunkHeader {
    format: CompressionFormat,
    compressed: usize,
    uncompressed: usize,
}

fn parse_chunk_header(bytes: &[u8], offset: usize) -> Result<ChunkHeader> {
    let malformed = |reason: &str| Error::MalformedChunk {
        offset,
        reason: reason.to_owned(),
    };
    let Some(tag) = read_u64(bytes, offset) else {
        return Err(malformed("truncated tag"));
    };
    if tag != CHUNK_TAG {
        return Err(malformed("bad tag"));
    }
    let Some(block) = read_u64(bytes, offset + 8) else {
        return Err(malformed("truncated block size"));
    };
    let Ok(block) = usize::try_from(block) else {
        return Err(malformed("block size overflow"));
    };
    if block != CHUNK_BLOCK {
        return Err(malformed("unexpected block size"));
    }
    let Some(&format) = bytes.get(offset + 16) else {
        return Err(malformed("truncated format byte"));
    };
    let sizes: Vec<u64> = (0..4)
        .map(|i| read_u64(bytes, offset + 17 + 8 * i))
        .collect::<Option<_>>()
        .ok_or_else(|| malformed("truncated size table"))?;
    let [total_comp, total_uncomp, comp, uncomp] = sizes[..] else {
        return Err(malformed("size table shape"));
    };
    if total_comp != comp || total_uncomp != uncomp {
        return Err(malformed("multi-block chunk (unsupported)"));
    }
    let (Ok(compressed), Ok(uncompressed)) = (usize::try_from(comp), usize::try_from(uncomp))
    else {
        return Err(malformed("size overflow"));
    };
    if uncompressed > CHUNK_BLOCK {
        return Err(malformed("uncompressed size exceeds block size"));
    }
    Ok(ChunkHeader {
        format: CompressionFormat::from(format),
        compressed,
        uncompressed,
    })
}

fn find_chunk_start(bytes: &[u8]) -> Option<usize> {
    let needle = CHUNK_MAGIC.to_le_bytes();
    memchr::memmem::find_iter(bytes, &needle)
        .find(|&candidate| parse_chunk_header(bytes, candidate).is_ok())
}

const MAX_ARCHIVE_VERSION: i32 = 64;

fn has_inner_header_signature(prefix: &[u8], offset: usize) -> bool {
    let Some(version) = read_i32(prefix, offset + 4) else {
        return false;
    };
    if !(1..=MAX_ARCHIVE_VERSION).contains(&version) {
        return false;
    }
    prefix.get(offset + 8..offset + 12) == Some(&[1, 0, 0, 0])
}

fn is_eof_relative(prefix: &[u8], offset: usize, file_len: usize) -> bool {
    let Some(value) = read_i32(prefix, offset) else {
        return false;
    };
    let (Ok(offset), Ok(file_len)) = (i64::try_from(offset), i64::try_from(file_len)) else {
        return false;
    };
    let value = i64::from(value);
    value >= MIN_SIZE_FIELD && offset + 4 + value == file_len
}

/// Locate the two EOF-relative size fields structurally.
///
/// The compressed-total field always occupies the last four bytes of the
/// prefix. The inner-archive field is immediately followed by the inner
/// header's version ints (a small archive version, then `01 00 00 00`;
/// observed `02` and `03` across game versions). Requiring both the
/// signature and the EOF relation rules out the coincidental matches a
/// relation-only scan occasionally produces.
fn find_eof_relative_fields(prefix: &[u8], file_len: usize) -> Result<[usize; 2]> {
    let compressed_total = prefix.len().saturating_sub(4);
    if !is_eof_relative(prefix, compressed_total, file_len) {
        return Err(Error::SizeFieldCount(0));
    }
    let inner: Vec<usize> = (0..prefix.len().saturating_sub(3))
        .filter(|&offset| {
            is_eof_relative(prefix, offset, file_len) && has_inner_header_signature(prefix, offset)
        })
        .collect();
    let [inner] = inner[..] else {
        return Err(Error::SizeFieldCount(inner.len() + 1));
    };
    Ok([inner, compressed_total])
}

impl SaveFile {
    /// Parse a raw `.sav` file.
    ///
    /// # Errors
    ///
    /// Returns an error if no valid chunk stream is found, a chunk is
    /// malformed or uses an unknown compression format, inflation fails, or
    /// the prefix does not contain exactly the two expected EOF-relative
    /// size fields.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let Some(start) = find_chunk_start(bytes) else {
            return Err(Error::NoChunkStream);
        };
        let prefix = bytes[..start].to_vec();
        let eof_relative_fields = find_eof_relative_fields(&prefix, bytes.len())?;
        let archive_version = read_i32(&prefix, eof_relative_fields[0] + 4)
            .map_or(ArchiveVersion::Other(0), ArchiveVersion::from);

        let mut body = Vec::new();
        let mut offset = start;
        while offset < bytes.len() {
            let ChunkHeader {
                format,
                compressed,
                uncompressed,
            } = parse_chunk_header(bytes, offset)?;
            match format {
                CompressionFormat::Zlib => {}
                CompressionFormat::Unknown(byte) => {
                    return Err(Error::UnsupportedCompression(byte));
                }
            }
            let data_start = offset + CHUNK_HEADER_LEN;
            let data_end = data_start
                .checked_add(compressed)
                .filter(|&e| e <= bytes.len());
            let Some(data_end) = data_end else {
                return Err(Error::OutOfBounds {
                    what: "chunk data",
                    value: data_start.saturating_add(compressed),
                    len: bytes.len(),
                });
            };
            let mut block = Vec::with_capacity(uncompressed);
            let mut decoder = ZlibDecoder::new(&bytes[data_start..data_end]);
            decoder
                .read_to_end(&mut block)
                .map_err(|source| Error::Inflate { offset, source })?;
            if block.len() != uncompressed {
                return Err(Error::MalformedChunk {
                    offset,
                    reason: format!(
                        "inflated {} bytes, header declared {uncompressed}",
                        block.len()
                    ),
                });
            }
            body.extend_from_slice(&block);
            offset = data_end;
        }
        Ok(Self {
            prefix,
            body,
            eof_relative_fields,
            archive_version,
        })
    }

    /// The inner-archive serialization version of this save.
    #[must_use]
    pub fn archive_version(&self) -> ArchiveVersion {
        self.archive_version
    }

    /// Rebuild a complete `.sav` file from the (possibly edited) body.
    ///
    /// Recompresses the body into 128 KiB zlib chunks and rewrites both
    /// EOF-relative size fields in the prefix so the result is internally
    /// consistent regardless of how the body length changed.
    ///
    /// # Panics
    ///
    /// Panics if the rebuilt file would exceed `i32::MAX` bytes; real saves
    /// are under a megabyte, so this indicates memory corruption, not data.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        use std::io::Write as _;

        use az::Az as _;

        let mut payload = Vec::new();
        for block in self.body.chunks(CHUNK_BLOCK) {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
            encoder.write_all(block).expect("write to Vec cannot fail");
            let compressed = encoder.finish().expect("zlib finish cannot fail");
            payload.extend_from_slice(&CHUNK_TAG.to_le_bytes());
            payload.extend_from_slice(&CHUNK_BLOCK.az::<u64>().to_le_bytes());
            payload.push(3);
            let comp_len = compressed.len().az::<u64>();
            let uncomp_len = block.len().az::<u64>();
            for size in [comp_len, uncomp_len, comp_len, uncomp_len] {
                payload.extend_from_slice(&size.to_le_bytes());
            }
            payload.extend_from_slice(&compressed);
        }

        let mut out = self.prefix.clone();
        out.extend_from_slice(&payload);
        for &offset in &self.eof_relative_fields {
            let value = out.len() - offset - 4;
            let value = i32::try_from(value).expect("save files are far below 2 GiB");
            out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        out
    }

    /// The plaintext prefix (header + metadata) preceding the chunk stream.
    #[must_use]
    pub fn prefix(&self) -> &[u8] {
        &self.prefix
    }

    /// Offsets within the prefix of the two EOF-relative `i32` size fields.
    ///
    /// These are rewritten by [`SaveFile::to_bytes`]; their stored values
    /// depend on the exact compressed payload length, so they should be
    /// excluded when comparing prefixes across a recompression roundtrip.
    #[must_use]
    pub fn size_field_offsets(&self) -> [usize; 2] {
        self.eof_relative_fields
    }

    /// Mutable access to the prefix for in-place metadata edits.
    ///
    /// Only same-length edits are safe here; the prefix's internal record
    /// sizes are not currently remapped.
    pub fn prefix_mut(&mut self) -> &mut [u8] {
        &mut self.prefix
    }
}
