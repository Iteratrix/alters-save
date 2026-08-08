import init, { summarize, apply_edits } from "./pkg/alters_save_web.js";

const KNOWN_ITEMS = [
  "BridgePylon", "RepairKit", "Recharger", "RadiationFilter", "RockDrill",
  "RockDrill_Charge", "Hook", "Outpost", "PylonComponent", "PylonComponent_Pack",
  "PolymersRefinery", "PlantationKit", "Flashlight", "QuantumCore",
  "Dumbbells", "Dumplings",
];

const el = (id) => document.getElementById(id);
const dropZone = el("drop-zone");
const editor = el("editor");
const statusLine = el("status");

const hasFsAccess = "showOpenFilePicker" in window;
el("fs-hint").textContent = hasFsAccess
  ? "Your browser can save directly back to the file after you grant permission."
  : "Your browser will download the edited file; move it back into your save folder.";

let state = null;

function floatFieldRow(name, value, onChange) {
  const row = document.createElement("div");
  row.className = "field";
  const label = document.createElement("label");
  label.textContent = name;
  const input = document.createElement("input");
  input.type = "number";
  input.min = "0";
  input.max = "1";
  input.step = "0.05";
  input.value = value.toFixed(3);
  input.addEventListener("input", () => {
    const parsed = Number.parseFloat(input.value);
    const changed = Number.isFinite(parsed) && Math.abs(parsed - value) > 1e-6;
    row.classList.toggle("changed", changed);
    onChange(changed ? parsed : null);
  });
  row.append(label, input);
  return row;
}

function setStatus(message, kind = "") {
  statusLine.textContent = message;
  statusLine.className = kind;
}

function fieldRow(name, value, onChange) {
  const row = document.createElement("div");
  row.className = "field";
  const label = document.createElement("label");
  label.textContent = name;
  const input = document.createElement("input");
  input.type = "number";
  input.min = "0";
  input.max = "9999";
  input.value = String(value);
  input.addEventListener("input", () => {
    const parsed = Number.parseInt(input.value, 10);
    const changed = Number.isFinite(parsed) && parsed !== value;
    row.classList.toggle("changed", changed);
    onChange(changed ? parsed : null);
  });
  row.append(label, input);
  return row;
}

function render() {
  const { summary } = state;
  el("file-name").textContent = state.fileName;
  el("file-meta").textContent =
    `archive v${summary.archive_version} · ${summary.resources.length} resources · ${summary.items.length} item stacks`;

  const timeDiv = el("time");
  timeDiv.replaceChildren();
  state.timeEdit = null;
  if (summary.time) {
    const clock = summary.time;
    const current = () => state.timeEdit ?? { ...clock };
    for (const unit of ["day", "hour", "minute"]) {
      timeDiv.append(fieldRow(unit, clock[unit], (edited) => {
        const next = current();
        next[unit] = edited === null ? clock[unit] : edited;
        state.timeEdit =
          next.day === clock.day && next.hour === clock.hour && next.minute === clock.minute
            ? null
            : next;
        updateSaveButton();
      }));
    }
  }

  const altersDiv = el("alters");
  altersDiv.replaceChildren();
  state.emotionEdits = [];
  state.radiationEdits = [];
  for (const alter of summary.alters) {
    const heading = document.createElement("h3");
    heading.textContent = alter.name.replace(/^Jan_/, "").replace(/_\d+$/, "");
    heading.className = "muted";
    const grid = document.createElement("div");
    grid.className = "grid";
    grid.append(floatFieldRow("Radiation", alter.radiation, (edited) => {
      state.radiationEdits = state.radiationEdits.filter((e) => e.alter !== alter.name);
      if (edited !== null) state.radiationEdits.push({ alter: alter.name, value: edited });
      updateSaveButton();
    }));
    for (const emotion of alter.emotions) {
      grid.append(floatFieldRow(emotion.name, emotion.value, (edited) => {
        state.emotionEdits = state.emotionEdits.filter(
          (e) => !(e.alter === alter.name && e.emotion === emotion.name),
        );
        if (edited !== null) {
          state.emotionEdits.push({ alter: alter.name, emotion: emotion.name, value: edited });
        }
        updateSaveButton();
      }));
    }
    altersDiv.append(heading, grid);
  }

  const researchRow = el("research-row");
  const researchInfo = el("research-info");
  const researchBox = el("research-complete");
  researchBox.checked = false;
  if (summary.research) {
    const { unlocked, discovered, missing } = summary.research;
    researchInfo.textContent =
      `${discovered} of ${unlocked} available technologies completed` +
      (missing.length ? ` - ${missing.length} remaining.` : ".");
    researchRow.hidden = !(summary.can_complete_research && missing.length > 0);
  } else {
    researchInfo.textContent = "Research state unavailable in this save.";
    researchRow.hidden = true;
  }

  const questsDiv = el("quests");
  questsDiv.replaceChildren();
  state.questEdits = new Map();
  summary.quests.forEach((quest, index) => {
    questsDiv.append(fieldRow(`${quest.name}`, quest.deadline_day, (edited) => {
      if (edited === null) state.questEdits.delete(index);
      else state.questEdits.set(index, edited);
      updateSaveButton();
    }));
  });

  const resourcesDiv = el("resources");
  resourcesDiv.replaceChildren();
  for (const { name, amount } of summary.resources) {
    resourcesDiv.append(fieldRow(name, amount, (edited) => {
      if (edited === null) state.resourceEdits.delete(name);
      else state.resourceEdits.set(name, edited);
      updateSaveButton();
    }));
  }

  const itemsDiv = el("items");
  itemsDiv.replaceChildren();
  const itemsNote = el("items-note");
  if (summary.items_error) {
    itemsNote.textContent = `Item stacks unavailable: ${summary.items_error}`;
    itemsNote.hidden = false;
  } else {
    itemsNote.hidden = true;
    for (const { name, count } of summary.items) {
      itemsDiv.append(fieldRow(name, count, (edited) => {
        if (edited === null) state.itemEdits.delete(name);
        else state.itemEdits.set(name, edited);
        updateSaveButton();
      }));
    }
  }

  const addRow = el("add-item-row");
  if (summary.can_add_items && !summary.items_error) {
    addRow.hidden = false;
    const select = el("add-item-select");
    select.replaceChildren();
    const owned = new Set(summary.items.map((item) => item.name));
    for (const name of KNOWN_ITEMS.filter((name) => !owned.has(name))) {
      const option = document.createElement("option");
      option.value = name;
      option.textContent = name;
      select.append(option);
    }
  } else {
    addRow.hidden = true;
    if (!summary.can_add_items) {
      itemsNote.textContent =
        "This save uses an older archive version: counts can be edited, but adding new item types is disabled. Load the save in the current game version and let it re-save to upgrade.";
      itemsNote.hidden = false;
    }
  }
  renderPendingAdds();

  el("save-btn").textContent = state.handle ? "Save back to file" : "Download edited save";
  editor.hidden = false;
  updateSaveButton();
}

function renderPendingAdds() {
  const list = el("pending-adds");
  list.replaceChildren();
  state.pendingAdds.forEach((add, index) => {
    const item = document.createElement("li");
    item.textContent = `+ ${add.count} × ${add.name}`;
    const remove = document.createElement("button");
    remove.type = "button";
    remove.textContent = "remove";
    remove.addEventListener("click", () => {
      state.pendingAdds.splice(index, 1);
      renderPendingAdds();
      updateSaveButton();
    });
    item.append(remove);
    list.append(item);
  });
}

function hasEdits() {
  return (
    state.resourceEdits.size > 0 ||
    state.itemEdits.size > 0 ||
    state.pendingAdds.length > 0 ||
    state.timeEdit !== null ||
    state.emotionEdits.length > 0 ||
    state.radiationEdits.length > 0 ||
    state.questEdits.size > 0 ||
    el("research-complete").checked
  );
}

function updateSaveButton() {
  el("save-btn").disabled = !hasEdits();
}

async function loadFile(file, handle) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let summary;
  try {
    summary = JSON.parse(summarize(bytes));
  } catch (error) {
    setStatus(`Could not read this file as an Alters world save: ${error}`, "error");
    return;
  }
  state = {
    fileName: file.name,
    originalBytes: bytes,
    handle: handle ?? null,
    summary,
    resourceEdits: new Map(),
    itemEdits: new Map(),
    pendingAdds: [],
    timeEdit: null,
    emotionEdits: [],
    radiationEdits: [],
    questEdits: new Map(),
  };
  setStatus("");
  render();
}

function buildEdits() {
  return JSON.stringify({
    resources: [...state.resourceEdits].map(([name, amount]) => ({ name, amount })),
    item_counts: [...state.itemEdits].map(([name, count]) => ({ name, count })),
    add_items: state.pendingAdds,
    time: state.timeEdit,
    alter_emotions: state.emotionEdits,
    alter_radiation: state.radiationEdits,
    complete_research: el("research-complete").checked,
    quest_deadlines: [...state.questEdits].map(([index, day]) => ({ index, day })),
  });
}

function download(bytes, name) {
  const blob = new Blob([bytes], { type: "application/octet-stream" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  anchor.click();
  URL.revokeObjectURL(url);
}

async function save() {
  let edited;
  try {
    edited = apply_edits(state.originalBytes, buildEdits());
  } catch (error) {
    setStatus(`Edit failed: ${error}`, "error");
    return;
  }
  if (state.handle) {
    try {
      const writable = await state.handle.createWritable();
      await writable.write(edited);
      await writable.close();
      setStatus("Saved back to the original file. A backup download is available above.", "ok");
    } catch (error) {
      setStatus(`Could not write file (${error}); downloading instead.`, "error");
      download(edited, state.fileName);
    }
  } else {
    download(edited, state.fileName);
    setStatus("Downloaded. Replace the original file in your save folder (keep a backup).", "ok");
  }
  await loadFile(new File([edited], state.fileName), state.handle);
}

async function openViaPicker() {
  if (hasFsAccess) {
    try {
      const [handle] = await window.showOpenFilePicker({
        types: [{ description: "The Alters save", accept: { "application/octet-stream": [".sav"] } }],
      });
      await loadFile(await handle.getFile(), handle);
      return;
    } catch (error) {
      if (error?.name === "AbortError") return;
    }
  }
  const input = document.createElement("input");
  input.type = "file";
  input.accept = ".sav";
  input.addEventListener("change", () => {
    if (input.files?.[0]) void loadFile(input.files[0]);
  });
  input.click();
}

dropZone.addEventListener("click", () => void openViaPicker());
dropZone.addEventListener("keydown", (event) => {
  if (event.key === "Enter" || event.key === " ") void openViaPicker();
});
dropZone.addEventListener("dragover", (event) => {
  event.preventDefault();
  dropZone.classList.add("drag");
});
dropZone.addEventListener("dragleave", () => dropZone.classList.remove("drag"));
dropZone.addEventListener("drop", async (event) => {
  event.preventDefault();
  dropZone.classList.remove("drag");
  const item = event.dataTransfer?.items?.[0];
  if (hasFsAccess && item?.getAsFileSystemHandle) {
    const handle = await item.getAsFileSystemHandle();
    if (handle?.kind === "file") {
      await loadFile(await handle.getFile(), handle);
      return;
    }
  }
  const file = event.dataTransfer?.files?.[0];
  if (file) void loadFile(file);
});

el("backup-btn").addEventListener("click", () => {
  download(state.originalBytes, state.fileName.replace(/\.sav$/, "") + ".backup.sav");
});
el("add-item-btn").addEventListener("click", () => {
  const name = el("add-item-select").value;
  const count = Number.parseInt(el("add-item-count").value, 10);
  if (!name || !Number.isFinite(count) || count < 1) return;
  state.pendingAdds.push({ name, count });
  renderPendingAdds();
  updateSaveButton();
});
el("research-complete").addEventListener("change", updateSaveButton);
el("save-btn").addEventListener("click", () => void save());
el("reset-btn").addEventListener("click", () => {
  void loadFile(new File([state.originalBytes], state.fileName), state.handle);
});

await init();
