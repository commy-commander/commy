import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { VOICE_FILTERS, voiceFilterKey, voiceFilterLabel, voiceFilterHeldHere, voiceFilterNote, voiceFilterNews } = require("../scripts/voice-filters.js");

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const read = (...path) => readFileSync(join(repo, ...path), "utf8");

test("the picker offers exactly the filters the voice pipeline knows, under the same names", () => {
  const filters = read("crates", "commy-audio", "src", "filters.rs");
  const backend = [...(/pub fn name\(self\)[\s\S]*?\n    \}/.exec(filters)[0]).matchAll(/=> "([a-z]+)"/g)].map(m => m[1]);
  assert.deepEqual(VOICE_FILTERS.map(f => f.key), backend);
  assert.equal(VOICE_FILTERS[0].key, "off");
  for (const filter of VOICE_FILTERS) {
    assert.ok(filter.label.length <= 8, `${filter.key} fits one button`);
    assert.ok(filter.sub && !filter.sub.includes("\n"));
  }
});

test("an unknown or missing choice reads as off", () => {
  assert.equal(voiceFilterKey({ voice_filter: "robot" }), "robot");
  assert.equal(voiceFilterKey({ voice_filter: "squeaky" }), "off");
  assert.equal(voiceFilterKey({}), "off");
  assert.equal(voiceFilterKey(null), "off");
  assert.equal(voiceFilterLabel("cave"), "Cave");
  assert.equal(voiceFilterLabel("nope"), "Off");
});

test("the held-back note shows only for the server you are in voice with", () => {
  const held = { held: true, server: "s1" };
  assert.equal(voiceFilterHeldHere(held, { id: "s1" }), true);
  assert.equal(voiceFilterHeldHere(held, { id: "s2" }), false);
  assert.equal(voiceFilterHeldHere(held, null), false, "out of voice nothing is held back");
  assert.equal(voiceFilterHeldHere({ held: false, server: "s1" }, { id: "s1" }), false);
  assert.equal(voiceFilterHeldHere(null, { id: "s1" }), false);
  assert.equal(voiceFilterHeldHere({ held: true, server: null }, { id: null, call: "u" }), false, "calls never hold a filter back");
});

test("the notes say who hears what", () => {
  assert.equal(voiceFilterNote("off", true, "Hangout"), "Everyone hears your own voice.");
  assert.match(voiceFilterNote("robot", true, "Hangout"), /^Hangout has voice filters switched off/);
  assert.match(voiceFilterNote("robot", false, "Hangout"), /Robot filter/);
  assert.match(voiceFilterNote("robot", true, null), /^This server/);
  assert.equal(voiceFilterNews(false, "Hangout"), "Your voice filter is back on");
  assert.match(voiceFilterNews(true, "Hangout"), /off in Hangout/);
});

test("the voice settings carry the picker and the app loads its script", () => {
  const html = read("apps", "desktop", "ui", "index.html");
  assert.ok(html.includes('<script src="scripts/voice-filters.js"></script>'));
  assert.ok(html.includes('class="settings-card native-only" id="voice-filter-card"'), "the browser, which cannot filter, hides it");
  assert.ok(html.includes('id="voice-filter"') && html.includes('id="voice-filter-note"'));
});
