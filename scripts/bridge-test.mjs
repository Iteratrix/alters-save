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

console.log("bridge tests passed");
