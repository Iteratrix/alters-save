//! Per-alter state: emotions and accumulated radiation.
//!
//! Each alter (instance names like `Jan_Technician_0`) serializes a mood
//! component whose embedded `BP_Emotion_<Name>_C_<id>` records each carry a
//! single `f32` after the standard float marker, and a `BP_CloneRadiation`
//! component with one `f32` the same way. Both are fixed-width in-place
//! edits, and the float-marker convention is identical in archive v2/v3.
//!
//! Ownership is attributed positionally: components serialize after their
//! owning actor, so each emotion/radiation record belongs to the nearest
//! preceding `Jan_<Class>_<n>` instance name in the body. This matches the
//! observed grouping in every save surveyed.

use memchr::memmem;

use crate::elb;
use crate::error::{Error, Result};

pub const EMOTION_NAMES: [&str; 8] = [
    "Fun",
    "Motivation",
    "Burden",
    "Gloom",
    "Frustration",
    "Rebellion",
    "Insecurity",
    "Anxiety",
];

/// One emotion value on one alter.
#[derive(Debug, Clone)]
pub struct Emotion {
    pub name: String,
    pub value: f32,
    value_offset: usize,
}

/// One alter's editable state.
#[derive(Debug, Clone)]
pub struct Alter {
    /// Instance name, e.g. `Jan_Technician_0`.
    pub name: String,
    /// Radiation `f32`s owned by this alter (usually one).
    pub radiation: Vec<(f32, usize)>,
    /// Emotion records owned by this alter. Emotions repeat (the mood
    /// system keeps several copies); edits should be applied to all
    /// records sharing a name.
    pub emotions: Vec<Emotion>,
}

fn owner_names(body: &[u8]) -> Vec<(usize, String)> {
    let mut owners = Vec::new();
    for start in memmem::find_iter(body, b"Jan_") {
        let tail = &body[start..(start + 48).min(body.len())];
        let end = tail
            .iter()
            .position(|&b| !(b.is_ascii_alphanumeric() || b == b'_'));
        let Some(end) = end else { continue };
        let Ok(name) = std::str::from_utf8(&tail[..end]) else {
            continue;
        };
        if name.len() > 4
            && name
                .rsplit('_')
                .next()
                .is_some_and(|n| n.parse::<u32>().is_ok())
        {
            owners.push((start, name.to_owned()));
        }
    }
    owners
}

fn owner_of(owners: &[(usize, String)], position: usize) -> Option<&str> {
    owners
        .iter()
        .take_while(|(start, _)| *start < position)
        .last()
        .map(|(_, name)| name.as_str())
}

fn float_after_marker(
    body: &[u8],
    content_start: usize,
    content_size: usize,
) -> Option<(f32, usize)> {
    let content = body.get(content_start..content_start + content_size)?;
    let marker = memmem::find(content, elb::FLOAT_MARKER)?;
    let value_offset = content_start + marker + elb::FLOAT_MARKER.len();
    Some((elb::read_f32(body, value_offset)?, value_offset))
}

/// Records shaped `[lstr class path][lstr instance][...][size][content]`,
/// tolerating both the top-level (`[i32 0]`) and component
/// (`[i32 1][lstr "Elb_Serializable"]`) shapes.
fn record_content_after(body: &[u8], after_class: usize) -> Option<(usize, usize)> {
    let (_instance, mut cursor) = elb::read_lstr(body, after_class)?;
    match elb::read_i32(body, cursor)? {
        0 => cursor += 4,
        1 => {
            cursor += 4;
            let (tag, after_tag) = elb::read_lstr(body, cursor)?;
            if tag != b"Elb_Serializable" {
                return None;
            }
            cursor = after_tag;
        }
        _ => return None,
    }
    let size = elb::read_usize(body, cursor)?;
    body.get(cursor + 4..(cursor + 4).checked_add(size)?)?;
    Some((cursor + 4, size))
}

fn index_of(result: &mut Vec<Alter>, owner: &str) -> usize {
    if let Some(index) = result.iter().position(|alter| alter.name == owner) {
        index
    } else {
        result.push(Alter {
            name: owner.to_owned(),
            radiation: Vec::new(),
            emotions: Vec::new(),
        });
        result.len() - 1
    }
}

/// Enumerate alters with their emotion and radiation fields.
///
/// # Errors
///
/// Returns an error only when the body contains emotion/radiation records
/// that cannot be attributed to any alter instance (unexpected layout).
pub fn alters(body: &[u8]) -> Result<Vec<Alter>> {
    let owners = owner_names(body);
    let mut result: Vec<Alter> = Vec::new();

    for emotion_name in EMOTION_NAMES {
        let class_path = format!(
            "/Game/P9Playable/Systems/Pawn/Emotions/BP_Emotion_{emotion_name}.BP_Emotion_{emotion_name}_C"
        );
        let needle = elb::lstr(class_path.as_bytes());
        for start in memmem::find_iter(body, &needle) {
            let Some((content_start, content_size)) =
                record_content_after(body, start + needle.len())
            else {
                continue;
            };
            let Some((value, value_offset)) = float_after_marker(body, content_start, content_size)
            else {
                continue;
            };
            let Some(owner) = owner_of(&owners, start) else {
                return Err(Error::MalformedChunk {
                    offset: start,
                    reason: format!("emotion {emotion_name} without preceding alter instance"),
                });
            };
            let owner = owner.to_owned();
            let index = index_of(&mut result, &owner);
            result[index].emotions.push(Emotion {
                name: emotion_name.to_owned(),
                value,
                value_offset,
            });
        }
    }

    let radiation_needle =
        elb::lstr(b"/Game/P9Playable/Systems/Pawn/Radiation/BP_CloneRadiation.BP_CloneRadiation_C");
    for start in memmem::find_iter(body, &radiation_needle) {
        let Some((content_start, content_size)) =
            record_content_after(body, start + radiation_needle.len())
        else {
            continue;
        };
        let Some((value, value_offset)) = float_after_marker(body, content_start, content_size)
        else {
            continue;
        };
        let Some(owner) = owner_of(&owners, start) else {
            continue;
        };
        let owner = owner.to_owned();
        let index = index_of(&mut result, &owner);
        result[index].radiation.push((value, value_offset));
    }

    Ok(result)
}

/// Set every emotion record named `emotion` on `alter` to `value`.
///
/// # Errors
///
/// Returns [`Error::OutOfBounds`] if any recorded offset does not fit.
pub fn set_emotion(body: &mut [u8], alter: &Alter, emotion: &str, value: f32) -> Result<()> {
    for record in alter.emotions.iter().filter(|e| e.name == emotion) {
        elb::write_f32(body, record.value_offset, value)?;
    }
    Ok(())
}

/// Set all radiation fields on `alter` to `value`.
///
/// # Errors
///
/// Returns [`Error::OutOfBounds`] if any recorded offset does not fit.
pub fn set_radiation(body: &mut [u8], alter: &Alter, value: f32) -> Result<()> {
    for &(_, offset) in &alter.radiation {
        elb::write_f32(body, offset, value)?;
    }
    Ok(())
}
