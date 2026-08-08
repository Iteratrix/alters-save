//! Game clock: the `P9TimeSystem` subsystem's `CurrentTime` struct.
//!
//! `CurrentTime` is a `StructProperty<P9DateTime>` holding `Date`, `Hour`,
//! and `Minute` tagged `IntProperty` values (plus an `Overflow` float we
//! leave alone). `Date` is the day number shown in-game and embedded in
//! save filenames. All three are fixed-width in-place edits.
//!
//! `P9DateTime` appears thousands of times in a save (timestamps on events
//! and components); only the one inside `P9TimeSystem`'s `CurrentTime`
//! property is the live clock, so lookup is anchored to that record.

use memchr::memmem;

use crate::elb;
use crate::error::{Error, Result};
use crate::sav::ArchiveVersion;

const TIME_SYSTEM: &[u8] = b"/Script/P9Playable.P9TimeSystem";
const CURRENT_TIME: &[u8] = b"\x0c\x00\x00\x00CurrentTime\x00";

/// The in-game clock with the offsets needed to rewrite it.
#[derive(Debug, Clone, Copy)]
pub struct GameTime {
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    day_offset: usize,
    hour_offset: usize,
    minute_offset: usize,
}

fn structure_error(reason: &str) -> Error {
    Error::MalformedChunk {
        offset: 0,
        reason: format!("time system structure: {reason}"),
    }
}

fn int_after(
    body: &[u8],
    window: (usize, usize),
    name: &str,
    version: ArchiveVersion,
) -> Result<(i32, usize)> {
    let (window_start, window_end) = window;
    let pattern = elb::int_prop_pattern(name, version);
    let region = &body[window_start..window_end];
    let found = memmem::find(region, &pattern)
        .ok_or_else(|| structure_error(&format!("{name} property not found")))?;
    let value_offset = window_start + found + pattern.len();
    let value =
        elb::read_i32(body, value_offset).ok_or_else(|| structure_error("truncated value"))?;
    Ok((value, value_offset))
}

/// Locate the live game clock.
///
/// # Errors
///
/// Returns an error when the `P9TimeSystem` record or the `CurrentTime`
/// fields cannot be found where expected.
pub fn game_time(body: &[u8], version: ArchiveVersion) -> Result<GameTime> {
    let (content_start, content_size, _) = elb::find_record(body, TIME_SYSTEM)
        .ok_or_else(|| structure_error("P9TimeSystem record not found"))?;
    let content_end = content_start + content_size;
    let region = &body[content_start..content_end];
    let current = memmem::find(region, CURRENT_TIME)
        .ok_or_else(|| structure_error("CurrentTime property not found"))?;
    let window = (
        content_start + current,
        content_end.min(content_start + current + 400),
    );

    let (day, day_offset) = int_after(body, window, "Date", version)?;
    let (hour, hour_offset) = int_after(body, window, "Hour", version)?;
    let (minute, minute_offset) = int_after(body, window, "Minute", version)?;
    Ok(GameTime {
        day,
        hour,
        minute,
        day_offset,
        hour_offset,
        minute_offset,
    })
}

/// Overwrite the clock in place.
///
/// # Errors
///
/// Returns [`Error::OutOfBounds`] if the recorded offsets do not fit the
/// supplied body.
pub fn set_game_time(
    body: &mut [u8],
    time: &GameTime,
    day: i32,
    hour: i32,
    minute: i32,
) -> Result<()> {
    let GameTime {
        day_offset,
        hour_offset,
        minute_offset,
        ..
    } = *time;
    elb::write_i32(body, day_offset, day)?;
    elb::write_i32(body, hour_offset, hour)?;
    elb::write_i32(body, minute_offset, minute)
}
