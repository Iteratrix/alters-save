//! Load-screen metadata in the plaintext prefix.
//!
//! The save-selection screen shows per-resource counts stored separately
//! from the body: after each `T_Resource_icon_<Name>` texture path comes a
//! version marker (`04 00 00 00 09 00 09 00`) and the displayed `i32`.
//! These are display-only - the game trusts the body on load - but leaving
//! them stale makes the load menu lie, so [`sync_counts`] mirrors the body's
//! container amounts into them after an edit.
//!
//! Icon names do not match resource class names one-to-one; the explicit
//! table below maps the known ones. Icons without a mapped container (and
//! containers without an icon, like `OrganicMatter`) are left untouched.

use memchr::memmem;

use crate::resources::{Container, ResourceName};

const VALUE_MARKER: &[u8] = &[0x04, 0, 0, 0, 0x09, 0, 0x09, 0];
const ICON_PREFIX: &[u8] = b"T_Resource_icon_";

fn container_name_for_icon(icon: &str) -> Option<&'static str> {
    match icon {
        "Metals" => Some("Metals"),
        "Rapidium" => Some("Rapidium"),
        "Minerals" => Some("Minerals"),
        "Mush" => Some("Mush"),
        "Polimer_icon" => Some("Polymers"),
        "FoodIcon" => Some("CookedMeal"),
        "Vegetables" => Some("RawFood"),
        _ => None,
    }
}

/// Mirror body container amounts into the prefix's load-screen counts.
///
/// Returns the number of icon entries updated. Unknown icons are skipped.
pub fn sync_counts(prefix: &mut [u8], containers: &[Container]) -> usize {
    let icon_offsets: Vec<usize> = memmem::find_iter(prefix, ICON_PREFIX).collect();
    let mut updated = 0;
    for offset in icon_offsets {
        let tail = &prefix[offset + ICON_PREFIX.len()..];
        let Some(end) = tail.iter().position(|&b| b == b'.' || b == b'\0') else {
            continue;
        };
        let Ok(icon) = std::str::from_utf8(&tail[..end]) else {
            continue;
        };
        let Some(container_name) = container_name_for_icon(icon) else {
            continue;
        };
        let Some(amount) = containers.iter().find_map(|container| {
            let Container {
                resource: ResourceName(name),
                amount,
                ..
            } = container;
            (name == container_name).then_some(*amount)
        }) else {
            continue;
        };
        let Some(terminator) = prefix[offset..].iter().position(|&b| b == b'\0') else {
            continue;
        };
        let value_offset = offset + terminator + 1 + VALUE_MARKER.len();
        let marker_start = offset + terminator + 1;
        if prefix.get(marker_start..marker_start + VALUE_MARKER.len()) != Some(VALUE_MARKER) {
            continue;
        }
        let Some(slot) = prefix.get_mut(value_offset..value_offset + 4) else {
            continue;
        };
        slot.copy_from_slice(&amount.to_le_bytes());
        updated += 1;
    }
    updated
}
