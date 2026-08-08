//! Research state: the `P9ResearchSubsystem`'s technology arrays.
//!
//! Two `ArrayProperty<ObjectProperty>` lists of `BP_Technology_*` class
//! paths matter:
//!
//! - `UnlockedResearches`: technologies visible/available in the tree.
//! - `DiscoveredTechnologies`: technologies actually completed
//!   (`Discovered ⊆ Unlocked` in every save surveyed).
//!
//! There is no per-technology "researched" bool - completing research
//! save-side means appending the missing class-path strings to the
//! `DiscoveredTechnologies` array payload and cascading four size fields:
//! the array's payload size, its element count, the subsystem record size,
//! and the whole-body size at offset 0 (the prefix EOF fields are handled
//! by [`crate::sav::SaveFile::to_bytes`]).
//!
//! v3 array header (after the two lstr type names and inner-type tree):
//!
//! ```text
//! [i32 0][u8 0][i32 payload_size][u8 0][i32 count][lstr element]*
//! ```
//!
//! Appending is restricted to archive v3, matching the item-injection
//! policy; v2 reads work through the same parse path.

use memchr::memmem;

use crate::elb;
use crate::error::{Error, Result};
use crate::sav::ArchiveVersion;

const SUBSYSTEM: &[u8] = b"/Script/P9Playable.P9ResearchSubsystem";

/// A technology class-path stem, e.g. `ModuleRecycling`
/// (`BP_Technology_<stem>_C`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TechName(pub String);

/// Parsed research state.
#[derive(Debug)]
pub struct Research {
    pub unlocked: Vec<String>,
    pub discovered: Vec<String>,
    record_size_offset: usize,
    array_size_offset: usize,
    count_offset: usize,
    elements_end: usize,
}

fn structure_error(reason: &str) -> Error {
    Error::MalformedChunk {
        offset: 0,
        reason: format!("research structure: {reason}"),
    }
}

struct ParsedArray {
    paths: Vec<String>,
    size_offset: usize,
    count_offset: usize,
    elements_end: usize,
}

fn parse_object_array(
    body: &[u8],
    content: (usize, usize),
    name: &str,
    version: ArchiveVersion,
) -> Result<ParsedArray> {
    let (content_start, content_size) = content;
    let region = &body[content_start..content_start + content_size];
    let mut needle = elb::lstr(name.as_bytes());
    needle.extend_from_slice(&elb::lstr(b"ArrayProperty"));
    let found = memmem::find(region, &needle)
        .ok_or_else(|| structure_error(&format!("{name} array not found")))?;
    let cursor = content_start + found + needle.len();

    let (size_offset, count_offset) = match version {
        ArchiveVersion::V3 | ArchiveVersion::Other(_) => {
            let children =
                elb::read_i32(body, cursor).ok_or_else(|| structure_error("array child count"))?;
            if children != 1 {
                return Err(structure_error("unexpected array type tree"));
            }
            let (inner, after_inner) = elb::read_lstr(body, cursor + 4)
                .ok_or_else(|| structure_error("array inner type"))?;
            if inner != b"ObjectProperty" {
                return Err(structure_error("array inner type is not ObjectProperty"));
            }
            if elb::read_i32(body, after_inner) != Some(0) {
                return Err(structure_error("array pre-size pad"));
            }
            let size_offset = after_inner + 4;
            if body.get(size_offset + 4) != Some(&0) {
                return Err(structure_error("array post-size flag"));
            }
            (size_offset, size_offset + 5)
        }
        ArchiveVersion::V2 => {
            let size_offset = cursor;
            if elb::read_i32(body, size_offset + 4) != Some(0) {
                return Err(structure_error("array post-size pad (v2)"));
            }
            let (inner, after_inner) = elb::read_lstr(body, size_offset + 8)
                .ok_or_else(|| structure_error("array inner type (v2)"))?;
            if inner != b"ObjectProperty" {
                return Err(structure_error(
                    "array inner type is not ObjectProperty (v2)",
                ));
            }
            if body.get(after_inner) != Some(&0) {
                return Err(structure_error("array flag (v2)"));
            }
            (size_offset, after_inner + 1)
        }
    };

    let payload_size =
        elb::read_usize(body, size_offset).ok_or_else(|| structure_error("array size"))?;
    let count = elb::read_usize(body, count_offset).ok_or_else(|| structure_error("count"))?;
    let payload_end = count_offset + payload_size;

    let mut paths = Vec::with_capacity(count);
    let mut element = count_offset + 4;
    for _ in 0..count {
        let (path, next) =
            elb::read_lstr(body, element).ok_or_else(|| structure_error("array element"))?;
        paths.push(String::from_utf8_lossy(path).into_owned());
        element = next;
    }
    if element != payload_end {
        return Err(structure_error("array payload size mismatch"));
    }
    Ok(ParsedArray {
        paths,
        size_offset,
        count_offset,
        elements_end: element,
    })
}

/// Parse the research subsystem.
///
/// # Errors
///
/// Returns an error when the subsystem record or either array deviates
/// from the expected layout.
pub fn research(body: &[u8], version: ArchiveVersion) -> Result<Research> {
    let (content_start, content_size, record_size_offset) = elb::find_record(body, SUBSYSTEM)
        .ok_or_else(|| structure_error("P9ResearchSubsystem record not found"))?;
    let unlocked = parse_object_array(
        body,
        (content_start, content_size),
        "UnlockedResearches",
        version,
    )?;
    let discovered = parse_object_array(
        body,
        (content_start, content_size),
        "DiscoveredTechnologies",
        version,
    )?;
    Ok(Research {
        unlocked: unlocked.paths,
        discovered: discovered.paths,
        record_size_offset,
        array_size_offset: discovered.size_offset,
        count_offset: discovered.count_offset,
        elements_end: discovered.elements_end,
    })
}

/// Technologies unlocked but not yet completed.
#[must_use]
pub fn missing(research: &Research) -> Vec<String> {
    let Research {
        unlocked,
        discovered,
        ..
    } = research;
    unlocked
        .iter()
        .filter(|path| !discovered.contains(path))
        .cloned()
        .collect()
}

/// Append the given technology paths to `DiscoveredTechnologies`,
/// returning the rebuilt body. Paths already discovered are skipped.
///
/// # Errors
///
/// Returns [`Error::UnsupportedArchiveVersion`] on non-v3 saves, or a
/// structure error if the arrays cannot be re-located.
///
/// # Panics
///
/// Panics only if internal length arithmetic overflows `i64`, which would
/// require a multi-gigabyte save body.
pub fn complete(body: &[u8], version: ArchiveVersion, paths: &[String]) -> Result<Vec<u8>> {
    match version {
        ArchiveVersion::V3 => {}
        ArchiveVersion::V2 => return Err(Error::UnsupportedArchiveVersion(2)),
        ArchiveVersion::Other(value) => return Err(Error::UnsupportedArchiveVersion(value)),
    }
    let state = research(body, version)?;
    let to_add: Vec<&String> = paths
        .iter()
        .filter(|path| !state.discovered.contains(path))
        .collect();
    if to_add.is_empty() {
        return Ok(body.to_vec());
    }

    let mut insertion = Vec::new();
    for path in &to_add {
        insertion.extend_from_slice(&elb::lstr(path.as_bytes()));
    }
    let delta = i64::try_from(insertion.len()).expect("insertion fits i64");
    let added = i64::try_from(to_add.len()).expect("count fits i64");

    let mut out = Vec::with_capacity(body.len() + insertion.len());
    out.extend_from_slice(&body[..state.elements_end]);
    out.extend_from_slice(&insertion);
    out.extend_from_slice(&body[state.elements_end..]);

    elb::adjust_i32(&mut out, state.array_size_offset, delta)?;
    elb::adjust_i32(&mut out, state.count_offset, added)?;
    elb::adjust_i32(&mut out, state.record_size_offset, delta)?;
    elb::adjust_i32(&mut out, 0, delta)?;
    Ok(out)
}
