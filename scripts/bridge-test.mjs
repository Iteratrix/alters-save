import { createRequire } from "node:module";
import fs from "node:fs";

const require = createRequire(import.meta.url);
const { summarize, apply_edits } = require("../target/wasm-node-test/alters_save_web.js");

function assert(condition, message) {
  if (!condition) {
    console.error(`FAIL: ${message}`);
    process.exit(1);
  }
}

const v3 = fs.readFileSync(new URL("../test-data/act0-day1.sav", import.meta.url));
const summary = JSON.parse(summarize(v3));
assert(summary.archive_version === "3", "act0 fixture is archive v3");
assert(summary.resources.length === 16, "16 resource containers");
assert(summary.can_add_items, "v3 allows item injection");

const edited = apply_edits(
  v3,
  JSON.stringify({
    resources: [{ name: "Metals", amount: 250 }],
    add_items: [{ name: "BridgePylon", count: 4 }],
  }),
);
const after = JSON.parse(summarize(Buffer.from(edited)));
assert(after.resources.find((r) => r.name === "Metals")?.amount === 250, "resource edit applied");
assert(after.items.find((i) => i.name === "BridgePylon")?.count === 4, "item injected");
assert(after.items.length === summary.items.length + 1, "one stack added");

const v2 = fs.readFileSync(new URL("../test-data/act2-day54.sav", import.meta.url));
const v2sum = JSON.parse(summarize(v2));
assert(v2sum.archive_version === "2", "act2 fixture is archive v2");
assert(!v2sum.can_add_items, "v2 blocks item injection");

let refused = false;
try {
  apply_edits(v2, JSON.stringify({ add_items: [{ name: "BridgePylon", count: 1 }] }));
} catch {
  refused = true;
}
assert(refused, "v2 injection refused");

const first = v2sum.items[0];
const v2edited = apply_edits(
  v2,
  JSON.stringify({ item_counts: [{ name: first.name, count: first.count + 3 }] }),
);
const v2after = JSON.parse(summarize(Buffer.from(v2edited)));
assert(
  v2after.items.find((i) => i.name === first.name)?.count === first.count + 3,
  "v2 count edit applied",
);

assert(summary.time?.day === 1, "day-1 fixture reports day 1");
assert(summary.research?.unlocked === 57, "57 unlocked techs on day 1");
assert(summary.quests.length > 0, "quests listed");

const timeEdited = apply_edits(
  v3,
  JSON.stringify({
    time: { day: 3, hour: 8, minute: 0 },
    complete_research: true,
  }),
);
const timeAfter = JSON.parse(summarize(Buffer.from(timeEdited)));
assert(timeAfter.time.day === 3 && timeAfter.time.hour === 8, "time edit applied");
assert(timeAfter.research.discovered === timeAfter.research.unlocked, "research completed");
assert(timeAfter.research.missing.length === 0, "no missing research");

const v2time = JSON.parse(summarize(v2));
assert(v2time.time?.day === 54, "v2 day 54");
assert(v2time.alters.length === 6, "6 alters in v2 save");
assert(v2time.alters[0].emotions.length === 8, "8 emotions per alter");
const alterName = v2time.alters[0].name;
const moodEdited = apply_edits(
  v2,
  JSON.stringify({
    alter_emotions: [{ alter: alterName, emotion: "Anxiety", value: 0 }],
    alter_radiation: [{ alter: alterName, value: 0 }],
    quest_deadlines: [{ index: 0, day: v2time.quests[0].deadline_day + 10 }],
  }),
);
const moodAfter = JSON.parse(summarize(Buffer.from(moodEdited)));
const editedAlter = moodAfter.alters.find((a) => a.name === alterName);
assert(
  editedAlter.emotions.find((e) => e.name === "Anxiety")?.value === 0,
  "v2 emotion edit applied",
);
assert(
  moodAfter.quests[0].deadline_day === v2time.quests[0].deadline_day + 10,
  "v2 quest deadline extended",
);

console.log("bridge tests passed");
