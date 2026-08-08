/// Errors produced while parsing or rebuilding a save file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No valid compressed chunk header was found anywhere in the file.
    ///
    /// `PlayerProfile.sav` and `playthrough_*.sav` use a different layout and
    /// currently parse to this error.
    #[error("no compressed chunk stream found (not a world save?)")]
    NoChunkStream,

    /// A chunk header was structurally invalid at the given file offset.
    #[error("malformed chunk header at offset {offset}: {reason}")]
    MalformedChunk { offset: usize, reason: String },

    /// A chunk declared a compression format this crate does not handle.
    #[error("unsupported compression format {0} (only zlib is known)")]
    UnsupportedCompression(u8),

    /// The zlib stream inside a chunk failed to inflate.
    #[error("zlib inflate failed at offset {offset}: {source}")]
    Inflate {
        offset: usize,
        source: std::io::Error,
    },

    /// The prefix did not contain exactly two EOF-relative size fields.
    ///
    /// Every known world save stores two `i32` fields in the plaintext prefix
    /// whose value equals `file_len - field_offset - 4` (the inner-archive
    /// size and the compressed-payload size). A different count means the
    /// format assumption is broken and rewriting would corrupt the file.
    #[error("expected 2 EOF-relative size fields in prefix, found {0}")]
    SizeFieldCount(usize),

    /// The save's inner-archive version is one this operation cannot handle
    /// safely (e.g. item injection is only verified against version 3).
    #[error("unsupported archive version {0} for this operation")]
    UnsupportedArchiveVersion(i32),

    /// A declared size or offset points outside the file.
    #[error("{what} out of bounds: {value} (file len {len})")]
    OutOfBounds {
        what: &'static str,
        value: usize,
        len: usize,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
