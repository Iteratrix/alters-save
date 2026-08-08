//! Base resource storage: the `P9ResourceSubsystem`'s sixteen
//! `P9ResourceContainer` records.
//!
//! Each container record looks like:
//!
//! ```text
//! [lstr "/Game/.../BP_Resource_<Name>.BP_Resource_<Name>_C"]
//! [i32 0]
//! [lstr "/Script/P9Playable.P9ResourceContainer"]
//! [lstr "P9ResourceContainer_<id>"]
//! [i32 0] [i32 payload_size] [u8 0]
//! [payload:
//!    lstr "None"
//!    [i32 0] [u8 0] [i32 4] [u16 9, u16 9] [i32 amount]
//!    [i32 4] [u16 9, u16 9] [i32 second]
//! ]
//! ```
//!
//! where `lstr` is `[i32 len incl NUL][bytes][NUL]`. The `04 00 00 00 09 00
//! 09 00` groups are version markers; the first `i32` after the first marker
//! is the stored amount. The second value looks like a capacity but is *not*
//! a clamp (saves exist with amount > second), so this module leaves it
//! untouched and only exposes it read-only.

use memchr::memmem;

use crate::error::{Error, Result};

const CONTAINER_SCRIPT: &[u8] = b"/Script/P9Playable.P9ResourceContainer\0";
const VALUE_MARKER: &[u8] = &[0x04, 0, 0, 0, 0x09, 0, 0x09, 0];
const CLASS_STEM: &[u8] = b"BP_Resource_";
const BACK_WINDOW: usize = 160;

/// A resource type name as it appears in class paths, e.g. `Metals`,
/// `Rapidium`, `OrganicMatter`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceName(pub String);

/// One resource container found in a save body.
#[derive(Debug, Clone)]
pub struct Container {
    pub resource: ResourceName,
    /// Stored amount in base storage.
    pub amount: i32,
    /// Second serialized value; capacity-like but not a clamp.
    pub second: i32,
    amount_offset: usize,
}

fn read_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    let slice = bytes.get(offset..offset.checked_add(4)?)?;
    let array: [u8; 4] = slice.try_into().ok()?;
    Some(i32::from_le_bytes(array))
}

fn read_lstr(bytes: &[u8], offset: usize) -> Option<(&[u8], usize)> {
    let len = read_i32(bytes, offset)?;
    let len = usize::try_from(len).ok()?;
    let start = offset.checked_add(4)?;
    let end = start.checked_add(len)?;
    let content = bytes.get(start..end)?;
    let (&0, text) = content.split_last()? else {
        return None;
    };
    Some((text, end))
}

fn resource_name_before(body: &[u8], script_offset: usize) -> Option<ResourceName> {
    let window_start = script_offset.saturating_sub(BACK_WINDOW);
    let window = &body[window_start..script_offset];
    let stem = memmem::rfind(window, CLASS_STEM)?;
    let tail = &window[stem + CLASS_STEM.len()..];
    let end = tail.iter().position(|&b| b == b'.' || b == b'\0')?;
    let name = std::str::from_utf8(&tail[..end]).ok()?;
    let name = name.strip_suffix("_C").unwrap_or(name);
    Some(ResourceName(name.to_owned()))
}

fn parse_container(body: &[u8], script_offset: usize) -> Option<Container> {
    let resource = resource_name_before(body, script_offset)?;
    let after_script = script_offset + CONTAINER_SCRIPT.len();
    let (_instance_name, after_name) = read_lstr(body, after_script)?;
    if read_i32(body, after_name)? != 0 {
        return None;
    }
    let payload_size = usize::try_from(read_i32(body, after_name + 4)?).ok()?;
    let payload_start = after_name + 9;
    let payload = body.get(payload_start..payload_start.checked_add(payload_size)?)?;

    let first = memmem::find(payload, VALUE_MARKER)?;
    let amount_in_payload = first + VALUE_MARKER.len();
    let amount = read_i32(payload, amount_in_payload)?;
    let rest = &payload[amount_in_payload + 4..];
    let second_marker = memmem::find(rest, VALUE_MARKER)?;
    let second = read_i32(rest, second_marker + VALUE_MARKER.len())?;

    Some(Container {
        resource,
        amount,
        second,
        amount_offset: payload_start + amount_in_payload,
    })
}

/// Find all resource containers in a decompressed save body.
///
/// Containers the parser cannot fully interpret are skipped rather than
/// reported as errors; a healthy world save yields sixteen.
#[must_use]
pub fn containers(body: &[u8]) -> Vec<Container> {
    memmem::find_iter(body, CONTAINER_SCRIPT)
        .filter_map(|offset| parse_container(body, offset))
        .collect()
}

/// Overwrite the stored amount of `container` in `body`.
///
/// This is a fixed-width in-place edit; no sizes change.
///
/// # Errors
///
/// Returns [`Error::OutOfBounds`] if the container's recorded offset does
/// not fit the supplied body (e.g. the body was rebuilt since the container
/// was found).
pub fn set_amount(body: &mut [u8], container: &Container, amount: i32) -> Result<()> {
    let Container { amount_offset, .. } = *container;
    let Some(slot) = body.get_mut(amount_offset..amount_offset + 4) else {
        return Err(Error::OutOfBounds {
            what: "resource amount field",
            value: amount_offset,
            len: body.len(),
        });
    };
    slot.copy_from_slice(&amount.to_le_bytes());
    Ok(())
}
