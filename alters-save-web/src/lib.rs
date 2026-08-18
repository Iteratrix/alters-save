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
use alters_save_core::{alters, meta, quests, research, resources, time};

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
struct TimeSummary {
    day: i32,
    hour: i32,
    minute: i32,
}

#[derive(Serialize)]
struct EmotionSummary {
    name: String,
    value: f32,
}

#[derive(Serialize)]
struct AlterSummary {
    name: String,
    radiation: f32,
    emotions: Vec<EmotionSummary>,
}

#[derive(Serialize)]
struct ResearchSummary {
    unlocked: usize,
    discovered: usize,
    missing: Vec<String>,
}

#[derive(Serialize)]
struct QuestSummary {
    name: String,
    deadline_day: i32,
}

#[derive(Serialize)]
struct Summary {
    archive_version: String,
    body_len: usize,
    resources: Vec<ResourceSummary>,
    items: Vec<StackSummary>,
    items_error: Option<String>,
    can_add_items: bool,
    time: Option<TimeSummary>,
    alters: Vec<AlterSummary>,
    dead_alters: Vec<String>,
    research: Option<ResearchSummary>,
    can_complete_research: bool,
    quests: Vec<QuestSummary>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Edits {
    resources: Vec<ResourceEdit>,
    item_counts: Vec<ItemCountEdit>,
    add_items: Vec<AddItemEdit>,
    time: Option<TimeEdit>,
    alter_emotions: Vec<EmotionEdit>,
    alter_radiation: Vec<RadiationEdit>,
    complete_research: bool,
    quest_deadlines: Vec<QuestDeadlineEdit>,
}

#[derive(Deserialize)]
struct TimeEdit {
    day: i32,
    hour: i32,
    minute: i32,
}

#[derive(Deserialize)]
struct EmotionEdit {
    alter: String,
    emotion: String,
    value: f32,
}

#[derive(Deserialize)]
struct RadiationEdit {
    alter: String,
    value: f32,
}

#[derive(Deserialize)]
struct QuestDeadlineEdit {
    index: usize,
    day: i32,
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
    let clock = time::game_time(&save.body, version)
        .ok()
        .map(|t| TimeSummary {
            day: t.day,
            hour: t.hour,
            minute: t.minute,
        });
    let roster = alters::alters(&save.body)
        .unwrap_or_default()
        .into_iter()
        .map(|alter| {
            let mut emotions: Vec<EmotionSummary> = Vec::new();
            for emotion in &alter.emotions {
                if !emotions.iter().any(|e| e.name == emotion.name) {
                    emotions.push(EmotionSummary {
                        name: emotion.name.clone(),
                        value: emotion.value,
                    });
                }
            }
            AlterSummary {
                name: alter.name.clone(),
                radiation: alter.radiation.first().map_or(0.0, |&(value, _)| value),
                emotions,
            }
        })
        .collect();
    let dead = alters::dead_alters(&save.body, version);
    let research_summary = research::research(&save.body, version).ok().map(|state| {
        let missing = research::missing(&state);
        ResearchSummary {
            unlocked: state.unlocked.len(),
            discovered: state.discovered.len(),
            missing,
        }
    });
    let quest_list = quests::deadlines(&save.body, version)
        .into_iter()
        .map(|quest| QuestSummary {
            name: quest.name,
            deadline_day: quest.deadline_day,
        })
        .collect();

    let summary = Summary {
        archive_version: version_label(version),
        body_len: save.body.len(),
        resources,
        items,
        items_error,
        can_add_items: version == ArchiveVersion::V3,
        time: clock,
        alters: roster,
        dead_alters: dead,
        research: research_summary,
        can_complete_research: version == ArchiveVersion::V3,
        quests: quest_list,
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

    if let Some(TimeEdit { day, hour, minute }) = &edits.time {
        let clock = time::game_time(&save.body, version).map_err(err)?;
        time::set_game_time(&mut save.body, &clock, *day, *hour, *minute).map_err(err)?;
    }

    if !edits.alter_emotions.is_empty() || !edits.alter_radiation.is_empty() {
        let roster = alters::alters(&save.body).map_err(err)?;
        for EmotionEdit {
            alter,
            emotion,
            value,
        } in &edits.alter_emotions
        {
            let target = roster
                .iter()
                .find(|candidate| &candidate.name == alter)
                .ok_or_else(|| err(format!("unknown alter: {alter}")))?;
            alters::set_emotion(&mut save.body, target, emotion, *value).map_err(err)?;
        }
        for RadiationEdit { alter, value } in &edits.alter_radiation {
            let target = roster
                .iter()
                .find(|candidate| &candidate.name == alter)
                .ok_or_else(|| err(format!("unknown alter: {alter}")))?;
            alters::set_radiation(&mut save.body, target, *value).map_err(err)?;
        }
    }

    if !edits.quest_deadlines.is_empty() {
        let deadline_list = quests::deadlines(&save.body, version);
        for QuestDeadlineEdit { index, day } in &edits.quest_deadlines {
            let quest = deadline_list
                .get(*index)
                .ok_or_else(|| err(format!("quest index out of range: {index}")))?;
            quests::set_deadline(&mut save.body, quest, *day).map_err(err)?;
        }
    }

    for AddItemEdit { name, count } in &edits.add_items {
        save.body =
            items::add_stack(&save.body, version, &ItemClass(name.clone()), *count).map_err(err)?;
    }

    if edits.complete_research {
        let state = research::research(&save.body, version).map_err(err)?;
        let missing = research::missing(&state);
        save.body = research::complete(&save.body, version, &missing).map_err(err)?;
    }

    let containers = resources::containers(&save.body);
    meta::sync_counts(save.prefix_mut(), &containers);
    Ok(save.to_bytes())
}
