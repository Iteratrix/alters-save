//! Quest deadlines: `QuestDeadlineInDay` fields on quest instances inside
//! the `P9QuestSubsystem` list.
//!
//! Each quest instance is an embedded `BP_<quest>_C` record carrying
//! `StartDay` / `DurationInDay` / `QuestDeadlineInDay` tagged
//! `IntProperty` values - all fixed-width in-place edits. Deadline values
//! are absolute day numbers. Quest *state* (completed/failed) lives in the
//! narrative event stream and is deliberately not touched here.
//!
//! Quests are named by the nearest preceding `/Game/P9Playable/Quest/...`
//! class path, which is how the instances are laid out in the list.

use memchr::memmem;

use crate::elb;
use crate::error::Result;
use crate::sav::ArchiveVersion;

/// One quest's deadline field.
#[derive(Debug, Clone)]
pub struct QuestDeadline {
    /// Quest class stem, e.g. `FindBase` (`BP_FindBase_C`).
    pub name: String,
    /// Absolute day number of the deadline.
    pub deadline_day: i32,
    value_offset: usize,
}

fn quest_class_names(body: &[u8]) -> Vec<(usize, String)> {
    let prefix = b"/Game/P9Playable/Quest/";
    let mut names = Vec::new();
    for start in memmem::find_iter(body, prefix) {
        let tail = &body[start..(start + 200).min(body.len())];
        let Some(end) = tail.iter().position(|&b| b == 0) else {
            continue;
        };
        let Ok(path) = std::str::from_utf8(&tail[..end]) else {
            continue;
        };
        let Some(class) = path.rsplit('.').next() else {
            continue;
        };
        let stem = class
            .strip_prefix("BP_")
            .unwrap_or(class)
            .strip_suffix("_C")
            .unwrap_or(class);
        names.push((start, stem.to_owned()));
    }
    names
}

/// Enumerate all quest deadline fields.
///
/// Fields that cannot be attributed to a quest class are reported under
/// the name `?`.
#[must_use]
pub fn deadlines(body: &[u8], version: ArchiveVersion) -> Vec<QuestDeadline> {
    let names = quest_class_names(body);
    let pattern = elb::int_prop_pattern("QuestDeadlineInDay", version);
    memmem::find_iter(body, &pattern)
        .filter_map(|found| {
            let value_offset = found + pattern.len();
            let deadline_day = elb::read_i32(body, value_offset)?;
            let name = names
                .iter()
                .take_while(|(start, _)| *start < found)
                .last()
                .map_or_else(|| "?".to_owned(), |(_, name)| name.clone());
            Some(QuestDeadline {
                name,
                deadline_day,
                value_offset,
            })
        })
        .collect()
}

/// Overwrite one quest's deadline day in place.
///
/// # Errors
///
/// Returns [`crate::Error::OutOfBounds`] if the recorded offset does not
/// fit the supplied body.
pub fn set_deadline(body: &mut [u8], quest: &QuestDeadline, day: i32) -> Result<()> {
    elb::write_i32(body, quest.value_offset, day)
}
