//! Self-checks used by the corpus runner and the integration tests.
//!
//! [`verify`] exercises the full public surface against one save: parse,
//! recompress-roundtrip, resource edit, meta sync, and (on archive v3)
//! item injection - asserting after each step that nothing else changed.

use crate::items::{self, ItemClass};
use crate::resources;
use crate::sav::SaveFile;
use crate::{meta, Error};

/// Result of verifying one file.
#[derive(Debug)]
pub enum Outcome {
    /// All checks passed; the string summarizes what was exercised.
    Pass(String),
    /// The file is not a chunked world save (profile/playthrough files).
    NotWorldSave,
    /// A check failed.
    Fail(String),
}

fn masked_prefix(save: &SaveFile) -> Vec<u8> {
    let mut prefix = save.prefix().to_vec();
    for offset in save.size_field_offsets() {
        prefix[offset..offset + 4].copy_from_slice(&[0; 4]);
    }
    prefix
}

fn check_injection(save: &SaveFile) -> Result<String, String> {
    let before = match items::inventory(&save.body, save.archive_version()) {
        Ok(inventory) => inventory,
        Err(error) => return Ok(format!("no injection ({error})")),
    };
    if before.stacks.is_empty() {
        return Ok("no injection (empty stack list)".to_owned());
    }

    let mut injected = save.clone();
    injected.body = match items::add_stack(
        &injected.body,
        injected.archive_version(),
        &ItemClass("BridgePylon".to_owned()),
        4,
    ) {
        Ok(body) => body,
        Err(error @ Error::UnsupportedArchiveVersion(_)) => {
            return Ok(format!("injection skipped ({error})"));
        }
        Err(error) => return Err(format!("injection failed: {error}")),
    };
    let rebuilt = injected.to_bytes();
    let reparsed =
        SaveFile::parse(&rebuilt).map_err(|error| format!("reparse after injection: {error}"))?;
    let after = items::inventory(&reparsed.body, reparsed.archive_version())
        .map_err(|error| format!("inventory after injection: {error}"))?;

    if after.stacks.len() != before.stacks.len() + 1 {
        return Err(format!(
            "stack count {} -> {}, expected +1",
            before.stacks.len(),
            after.stacks.len()
        ));
    }
    if !after
        .stacks
        .iter()
        .any(|stack| stack.item.0 == "BridgePylon" && stack.count == 4)
    {
        return Err("injected BridgePylon x4 not found after roundtrip".to_owned());
    }
    let preserved = before.stacks.iter().all(|old| {
        after
            .stacks
            .iter()
            .any(|new| new.item == old.item && new.count == old.count)
    });
    if !preserved {
        return Err("pre-existing stacks changed during injection".to_owned());
    }
    if resources::containers(&reparsed.body).len() != resources::containers(&save.body).len() {
        return Err("resource containers disturbed by injection".to_owned());
    }
    Ok(format!("injection ok ({} stacks)", after.stacks.len()))
}

/// Run the full check battery against raw save bytes.
#[must_use]
pub fn verify(bytes: &[u8]) -> Outcome {
    let save = match SaveFile::parse(bytes) {
        Ok(save) => save,
        Err(Error::NoChunkStream) => return Outcome::NotWorldSave,
        Err(error) => return Outcome::Fail(format!("parse: {error}")),
    };
    let containers = resources::containers(&save.body);

    let rebuilt = save.to_bytes();
    let reparsed = match SaveFile::parse(&rebuilt) {
        Ok(reparsed) => reparsed,
        Err(error) => return Outcome::Fail(format!("reparse of rebuilt file: {error}")),
    };
    if reparsed.body != save.body {
        return Outcome::Fail("body changed across rebuild roundtrip".to_owned());
    }
    if masked_prefix(&reparsed) != masked_prefix(&save) {
        return Outcome::Fail("prefix changed across rebuild roundtrip".to_owned());
    }
    if reparsed.size_field_offsets() != save.size_field_offsets() {
        return Outcome::Fail("size field offsets moved across rebuild roundtrip".to_owned());
    }

    let mut edited = save.clone();
    let Some(first) = resources::containers(&edited.body).into_iter().next() else {
        return Outcome::Pass(format!(
            "body {} bytes, 0 resource containers, roundtrip ok",
            save.body.len()
        ));
    };
    if let Err(error) = resources::set_amount(&mut edited.body, &first, first.amount + 1) {
        return Outcome::Fail(format!("set_amount: {error}"));
    }
    let edited_containers = resources::containers(&edited.body);
    let synced = meta::sync_counts(edited.prefix_mut(), &edited_containers);
    let edited_bytes = edited.to_bytes();
    let edited_reparsed = match SaveFile::parse(&edited_bytes) {
        Ok(reparsed) => reparsed,
        Err(error) => return Outcome::Fail(format!("reparse of edited file: {error}")),
    };
    let echoed = resources::containers(&edited_reparsed.body)
        .into_iter()
        .next()
        .map(|container| container.amount);
    if echoed != Some(first.amount + 1) {
        return Outcome::Fail(format!("edit did not survive roundtrip: {echoed:?}"));
    }

    match check_injection(&save) {
        Ok(injection) => Outcome::Pass(format!(
            "body {} bytes, {} resource containers, {synced} meta counts, {injection}",
            save.body.len(),
            containers.len()
        )),
        Err(reason) => Outcome::Fail(format!("item injection: {reason}")),
    }
}
