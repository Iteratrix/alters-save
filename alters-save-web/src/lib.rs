//! WASM bridge: stateless JSON-in/bytes-out API over `alters-save-core`.
//!
//! Two entry points keep the JS side trivial:
//! - [`summarize`] parses a save and returns a JSON description of what is
//!   editable (resources, item stacks, archive version, capability flags).
//! - [`apply_edits`] parses, applies a JSON edit set, and returns the
//!   rebuilt file bytes. The original bytes are never mutated, so the page
//!   can always offer them as a backup.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use alters_save_core::items::{self, ItemClass};
use alters_save_core::sav::{ArchiveVersion, SaveFile};
use alters_save_core::{meta, resources};

#[derive(Serialize)]
struct ResourceSummary {
    name: String,
    amount: i32,
}

#[derive(Serialize)]
struct StackSummary {
    name: String,
    count: i32,
}

#[derive(Serialize)]
struct Summary {
    archive_version: String,
    body_len: usize,
    resources: Vec<ResourceSummary>,
    items: Vec<StackSummary>,
    items_error: Option<String>,
    can_add_items: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Edits {
    resources: Vec<ResourceEdit>,
    item_counts: Vec<ItemCountEdit>,
    add_items: Vec<AddItemEdit>,
}

#[derive(Deserialize)]
struct ResourceEdit {
    name: String,
    amount: i32,
}

#[derive(Deserialize)]
struct ItemCountEdit {
    name: String,
    count: i32,
}

#[derive(Deserialize)]
struct AddItemEdit {
    name: String,
    count: i32,
}

fn version_label(version: ArchiveVersion) -> String {
    match version {
        ArchiveVersion::V2 => "2".to_owned(),
        ArchiveVersion::V3 => "3".to_owned(),
        ArchiveVersion::Other(value) => format!("unknown ({value})"),
    }
}

fn err(message: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&message.to_string())
}

/// Install the panic hook so Rust panics surface in the browser console.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Parse a save and describe its editable contents as JSON.
///
/// # Errors
///
/// Returns a JS string error when the file is not a parseable world save.
#[wasm_bindgen]
pub fn summarize(bytes: &[u8]) -> Result<String, JsValue> {
    let save = SaveFile::parse(bytes).map_err(err)?;
    let version = save.archive_version();
    let resources = resources::containers(&save.body)
        .into_iter()
        .map(|container| ResourceSummary {
            name: container.resource.0,
            amount: container.amount,
        })
        .collect();
    let (items, items_error) = match items::inventory(&save.body, version) {
        Ok(inventory) => (
            inventory
                .stacks
                .into_iter()
                .map(|stack| StackSummary {
                    name: stack.item.0,
                    count: stack.count,
                })
                .collect(),
            None,
        ),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let summary = Summary {
        archive_version: version_label(version),
        body_len: save.body.len(),
        resources,
        items,
        items_error,
        can_add_items: version == ArchiveVersion::V3,
    };
    serde_json::to_string(&summary).map_err(err)
}

/// Apply a JSON edit set and return the rebuilt save bytes.
///
/// # Errors
///
/// Returns a JS string error when parsing fails, an edit references an
/// unknown resource or stack, or item injection is unsupported for the
/// save's archive version.
#[wasm_bindgen]
pub fn apply_edits(bytes: &[u8], edits_json: &str) -> Result<Vec<u8>, JsValue> {
    let edits: Edits = serde_json::from_str(edits_json).map_err(err)?;
    let mut save = SaveFile::parse(bytes).map_err(err)?;
    let version = save.archive_version();

    for ResourceEdit { name, amount } in &edits.resources {
        let container = resources::containers(&save.body)
            .into_iter()
            .find(|container| &container.resource.0 == name)
            .ok_or_else(|| err(format!("unknown resource: {name}")))?;
        resources::set_amount(&mut save.body, &container, *amount).map_err(err)?;
    }

    for ItemCountEdit { name, count } in &edits.item_counts {
        let inventory = items::inventory(&save.body, version).map_err(err)?;
        let stack = inventory
            .stacks
            .iter()
            .find(|stack| &stack.item.0 == name)
            .ok_or_else(|| err(format!("unknown item stack: {name}")))?;
        items::set_count(&mut save.body, stack, *count).map_err(err)?;
    }

    for AddItemEdit { name, count } in &edits.add_items {
        save.body =
            items::add_stack(&save.body, version, &ItemClass(name.clone()), *count).map_err(err)?;
    }

    let containers = resources::containers(&save.body);
    meta::sync_counts(save.prefix_mut(), &containers);
    Ok(save.to_bytes())
}
