//! Shared low-level helpers for the Elb body serialization.
//!
//! `lstr` is the pervasive string encoding: `[i32 len incl NUL][bytes][NUL]`.
//! Numeric property values sit behind fixed version-marker sequences that
//! are identical across archive v2 and v3:
//!
//! ```text
//! int:   04 00 00 00 09 00 09 00 [i32]
//! float: 04 00 00 00 15 00 15 00 [f32]
//! ```
//!
//! Tagged property headers differ between versions after the type name(s):
//!
//! ```text
//! v3: [lstr name][lstr type][type tree...] [i32 0][i32 size][u8 flag] payload
//! v2: [lstr name][lstr type] [i32 size][i32 0][child lstrs][u8 flag] payload
//! ```
//!
//! so an `IntProperty` value sits behind `00*4 04 00 00 00 00` in v3 and
//! `04 00 00 00 00 00 00 00 00` in v2.

use crate::sav::ArchiveVersion;

pub(crate) const FLOAT_MARKER: &[u8] = &[0x04, 0, 0, 0, 0x15, 0, 0x15, 0];

pub(crate) fn read_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    let slice = bytes.get(offset..offset.checked_add(4)?)?;
    let array: [u8; 4] = slice.try_into().ok()?;
    Some(i32::from_le_bytes(array))
}

pub(crate) fn read_f32(bytes: &[u8], offset: usize) -> Option<f32> {
    let slice = bytes.get(offset..offset.checked_add(4)?)?;
    let array: [u8; 4] = slice.try_into().ok()?;
    Some(f32::from_le_bytes(array))
}

pub(crate) fn read_usize(bytes: &[u8], offset: usize) -> Option<usize> {
    usize::try_from(read_i32(bytes, offset)?).ok()
}

pub(crate) fn read_lstr(bytes: &[u8], offset: usize) -> Option<(&[u8], usize)> {
    let len = read_usize(bytes, offset)?;
    let start = offset.checked_add(4)?;
    let end = start.checked_add(len)?;
    let content = bytes.get(start..end)?;
    let (&0, text) = content.split_last()? else {
        return None;
    };
    Some((text, end))
}

pub(crate) fn lstr(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 5);
    let len = i32::try_from(text.len() + 1).expect("string length fits i32");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(text);
    out.push(0);
    out
}

/// Byte pattern of a tagged `IntProperty` named `name`, up to (excluding)
/// the value, for the given archive version.
pub(crate) fn int_prop_pattern(name: &str, version: ArchiveVersion) -> Vec<u8> {
    let mut out = lstr(name.as_bytes());
    out.extend_from_slice(&lstr(b"IntProperty"));
    match version {
        ArchiveVersion::V3 | ArchiveVersion::Other(_) => {
            out.extend_from_slice(&[0, 0, 0, 0, 4, 0, 0, 0, 0]);
        }
        ArchiveVersion::V2 => {
            out.extend_from_slice(&[4, 0, 0, 0, 0, 0, 0, 0, 0]);
        }
    }
    out
}

pub(crate) fn write_i32(buffer: &mut [u8], offset: usize, value: i32) -> crate::Result<()> {
    let Some(slot) = buffer.get_mut(offset..offset + 4) else {
        return Err(crate::Error::OutOfBounds {
            what: "i32 field",
            value: offset,
            len: buffer.len(),
        });
    };
    slot.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

pub(crate) fn write_f32(buffer: &mut [u8], offset: usize, value: f32) -> crate::Result<()> {
    let Some(slot) = buffer.get_mut(offset..offset + 4) else {
        return Err(crate::Error::OutOfBounds {
            what: "f32 field",
            value: offset,
            len: buffer.len(),
        });
    };
    slot.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// Adjust an `i32` size field by `delta`, checking bounds and overflow.
pub(crate) fn adjust_i32(buffer: &mut [u8], offset: usize, delta: i64) -> crate::Result<()> {
    let current = read_i32(buffer, offset).ok_or(crate::Error::OutOfBounds {
        what: "size field",
        value: offset,
        len: buffer.len(),
    })?;
    let adjusted =
        i32::try_from(i64::from(current) + delta).map_err(|_| crate::Error::OutOfBounds {
            what: "adjusted size field",
            value: offset,
            len: buffer.len(),
        })?;
    write_i32(buffer, offset, adjusted)
}

/// Locate a top-level record `[lstr class][lstr inst][i32 0][i32 size]`
/// by its `/Script/...` class path. Returns `(content_start,
/// content_size, size_field_offset)`.
///
/// The class-path string can also occur in the body as a mere reference,
/// so every occurrence is validated: the instance name must start with the
/// class's short name (e.g. `P9TimeSystem_` for
/// `/Script/P9Playable.P9TimeSystem`) and the record framing must be
/// well-formed.
pub(crate) fn find_record(body: &[u8], class_path: &[u8]) -> Option<(usize, usize, usize)> {
    let needle = lstr(class_path);
    let short_name = class_path.rsplit(|&b| b == b'.').next()?;
    let mut instance_prefix = short_name.to_vec();
    instance_prefix.push(b'_');

    memchr::memmem::find_iter(body, &needle).find_map(|start| {
        let after_class = start + needle.len();
        let (instance, after_name) = read_lstr(body, after_class)?;
        if !instance.starts_with(&instance_prefix) {
            return None;
        }
        if read_i32(body, after_name)? != 0 {
            return None;
        }
        let size_offset = after_name + 4;
        let size = read_usize(body, size_offset)?;
        body.get(size_offset + 4..size_offset.checked_add(4)?.checked_add(size)?)?;
        Some((size_offset + 4, size, size_offset))
    })
}
