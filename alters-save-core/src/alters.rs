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
use crate::sav::ArchiveVersion;

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

const CHARACTER_MANAGER: &[u8] = b"/Script/P9Playable.P9CharacterManagerSubsystem";

/// A dead alter from `P9CharacterManagerSubsystem.DeadAlters`, with its death
/// time (in-game P9DateTime) when it can be read.
#[derive(Debug, Clone, Default)]
pub struct DeadAlter {
    pub name: String,
    pub day: Option<i32>,
    pub hour: Option<i32>,
    pub minute: Option<i32>,
}

fn find_named_property(body: &[u8], start: usize, end: usize, name: &[u8]) -> Option<usize> {
    let needle = elb::lstr(name);
    let window = body.get(start..end)?;
    let rel = memmem::find(window, &needle)?;
    let tag = start + rel;
    let (found, _) = elb::read_lstr(body, tag)?;
    (found == name).then_some(tag)
}

fn skip_type_tree(body: &[u8], mut pos: usize, typ: &[u8], version: &ArchiveVersion) -> usize {
    let adv = |p: usize| if *version == ArchiveVersion::V3 { p + 4 } else { p };
    if typ == b"ArrayProperty" {
        pos = adv(pos);
        if let Some((inner, after)) = elb::read_lstr(body, pos) {
            pos = after;
            if inner == b"StructProperty" {
                pos = adv(pos);
                if let Some((_, after)) = elb::read_lstr(body, pos) {
                    pos = after;
                }
                pos = adv(pos);
                if let Some((_, after)) = elb::read_lstr(body, pos) {
                    pos = after;
                }
            }
        }
    } else if typ == b"StructProperty" {
        pos = adv(pos);
        if let Some((_, after)) = elb::read_lstr(body, pos) {
            pos = after;
        }
        pos = adv(pos);
        if let Some((_, after)) = elb::read_lstr(body, pos) {
            pos = after;
        }
    }
    pos
}

fn property_payload(body: &[u8], tag: usize, version: ArchiveVersion) -> Option<usize> {
    let (_name, after_name) = elb::read_lstr(body, tag)?;
    let (typ, mut cursor) = elb::read_lstr(body, after_name)?;
    match version {
        ArchiveVersion::V2 => {
            cursor += 8; // [i32 size][i32 0]
            cursor = skip_type_tree(body, cursor, typ, &version);
        }
        _ => {
            cursor = skip_type_tree(body, cursor, typ, &version);
            cursor += 8; // [i32 0][i32 size]
        }
    }
    Some(cursor + 1) // [u8 flag] then payload
}

/// Names of alters that have died, read from
/// `P9CharacterManagerSubsystem.DeadAlters`. Returns an empty list when the
/// record is absent or unparseable.
fn property_tag(
    body: &[u8],
    tag: usize,
    version: ArchiveVersion,
) -> Option<(Vec<u8>, Vec<u8>, usize)> {
    let (name, after_name) = elb::read_lstr(body, tag)?;
    let (typ, _) = elb::read_lstr(body, after_name)?;
    let payload = property_payload(body, tag, version)?;
    Some((name.to_vec(), typ.to_vec(), payload))
}

fn parse_datetime(body: &[u8], tag: usize, version: ArchiveVersion) -> Option<(i32, i32, i32)> {
    let payload = property_payload(body, tag, version)?;
    let mut day = None;
    let mut hour = None;
    let mut minute = None;
    let mut pos = payload;
    for _ in 0..8 {
        let (name, ftype, p) = property_tag(body, pos, version)?;
        if name == b"None" || ftype != b"IntProperty" {
            break;
        }
        let value = elb::read_i32(body, p)?;
        match name.as_slice() {
            b"Date" => day = Some(value),
            b"Hour" => hour = Some(value),
            b"Minute" => minute = Some(value),
            _ => {}
        }
        pos = p + 4;
    }
    Some((day?, hour?, minute?))
}

pub fn dead_alters(body: &[u8], version: ArchiveVersion) -> Vec<DeadAlter> {
    let Some((payload_start, payload_size, _)) = elb::find_record(body, CHARACTER_MANAGER) else {
        return Vec::new();
    };
    let payload_end = payload_start + payload_size;
    let Some(tag) = find_named_property(body, payload_start, payload_end, b"DeadAlters") else {
        return Vec::new();
    };
    let Some(payload) = property_payload(body, tag, version) else {
        return Vec::new();
    };
    let Some(region) = body.get(payload..payload_end) else {
        return Vec::new();
    };
    // Each dead alter's Name field is an lstr "Jan <Role>" (space-separated,
    // unlike the alive clones' underscore instance names); its DeathTime is a
    // P9DateTime. Collect both in order and pair them by index.
    let mut names: Vec<String> = Vec::new();
    for start in memmem::find_iter(region, b"Jan ") {
        let from = start;
        let to = (start + 4 + 32).min(region.len());
        let tail = &region[from..to];
        let end = tail
            .iter()
            .position(|&b| !(b.is_ascii_alphabetic() || b == b' '))
            .unwrap_or(tail.len());
        let name = String::from_utf8_lossy(&tail[..end]).into_owned();
        if !name.is_empty() {
            names.push(name);
        }
    }
    let mut times: Vec<(i32, i32, i32)> = Vec::new();
    let mut search = 0;
    while let Some(rel) = memmem::find(&region[search..], b"DeathTime") {
        let abs = search + rel;
        let tag_pos = payload + abs - 4;
        if let Some((name, typ, _)) = property_tag(body, tag_pos, version) {
            if name == b"DeathTime" && typ == b"StructProperty" {
                if let Some(dt) = parse_datetime(body, tag_pos, version) {
                    times.push(dt);
                }
            }
        }
        search = abs + 1;
    }
    names
        .into_iter()
        .zip(times.into_iter().chain(std::iter::repeat((0, 0, 0))))
        .map(|(name, (day, hour, minute))| DeadAlter {
            name,
            day: (day > 0).then_some(day),
            hour: (hour > 0).then_some(hour),
            minute: (minute > 0).then_some(minute),
        })
        .collect()
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
