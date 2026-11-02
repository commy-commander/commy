import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  CLEAR_ALL_WORD,
  USAGE_PARTS,
  formatBytes,
  describeUsage,
  describeFolder,
  describeMovePlan,
  describeClearedCache,
  clearAllConfirmed,
} = require("../scripts/data-folder.js");

const backend = readFileSync(join(import.meta.dirname, "..", "..", "src-tauri", "src", "data_location.rs"), "utf8");

test("every part the backend measures has a label, and nothing else does", () => {
  const measured = [...backend.matchAll(/UsagePart \{ id: "([a-z_]+)"/g)].map(m => m[1]);
  assert.deepEqual([...measured].sort(), USAGE_PARTS.map(part => part.id).sort());
});

test("the word to type matches the one the backend checks", () => {
  assert.equal(/CLEAR_ALL_WORD: &str = "([A-Z]+)"/.exec(backend)[1], CLEAR_ALL_WORD);
  assert.ok(clearAllConfirmed(" DELETE "));
  assert.ok(!clearAllConfirmed("delete"));
  assert.ok(!clearAllConfirmed(""));
});

test("sizes read like people write them", () => {
  assert.equal(formatBytes(0), "0 bytes");
  assert.equal(formatBytes(1536), "1.5 KB");
  assert.equal(formatBytes(3 * 1024 ** 3), "3.0 GB");
});

test("usage keeps the parts in a fixed order and drops empty ones", () => {
  const view = describeUsage({
    total: 400,
    free: 1024 ** 3,
    parts: [
      { id: "other", bytes: 100 },
      { id: "messages", bytes: 200 },
      { id: "logs", bytes: 0 },
      { id: "web_cache", bytes: 100 },
    ],
  });
  assert.deepEqual(view.rows.map(row => row.id), ["messages", "web_cache", "other"]);
  assert.equal(view.rows[0].share, 0.5);
  assert.equal(view.totalText, "400 bytes");
  assert.equal(view.freeText, "1.0 GB free on this drive");
  assert.equal(view.cacheText, "100 bytes");
});

test("usage without a free space answer leaves it out", () => {
  assert.equal(describeUsage({ total: 0, free: null, parts: [] }).freeText, "");
});

test("the folder note says where the folder came from", () => {
  const standard = describeFolder({ folder: "/data", kind: "default", can_move: true });
  assert.equal(standard.canUseStandard, false);
  assert.match(standard.note, /standard/);
  const chosen = describeFolder({ folder: "/games/Commy", kind: "chosen", can_move: true });
  assert.equal(chosen.canUseStandard, true);
  const bench = describeFolder({ folder: "/bench", kind: "overridden", can_move: false });
  assert.equal(bench.canMove, false);
  assert.match(bench.note, /COMMY_DATA_DIR/);
});

test("a move explains what happens and what keeps the data safe", () => {
  const plan = describeMovePlan({ target: "D:\\Games\\Commy", to_default: false, bytes: 2 * 1024 ** 3 }, { inCall: true });
  assert.equal(plan.title, "Move your data?");
  assert.match(plan.lines[0], /copy 2\.0 GB to D:\\Games\\Commy/);
  assert.match(plan.lines[1], /only after the new one checks out/);
  assert.match(plan.lines[2], /voice channel/);
  const back = describeMovePlan({ target: "/home", to_default: true, bytes: 10 });
  assert.match(back.title, /standard folder/);
  assert.equal(back.lines.length, 2);
});

test("clearing the cache says what is left for the next start", () => {
  assert.equal(describeClearedCache({ freed: 2048, on_restart: 0 }), "Cleared 2.0 KB.");
  assert.equal(describeClearedCache({ freed: 0, on_restart: 0 }), "The cache was already empty.");
  assert.match(describeClearedCache({ freed: 1024, on_restart: 1024 ** 2 }), /Another 1\.0 MB clears the next time Commy starts/);
});
