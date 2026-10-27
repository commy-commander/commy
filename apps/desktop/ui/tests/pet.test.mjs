import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { PET_SUITS, PET_NAME_MAX, PET_CARES, petMood, petCareWait, petCareDone, petProgress, petCleanName, petSvg } = require("../scripts/pet.js");

const core = readFileSync(join(import.meta.dirname, "..", "..", "..", "..", "crates", "commy-core", "src", "pet.rs"), "utf8");
const HOUR = 3600 * 1000;
const now = 100 * HOUR;
const pet = (last = {}, extra = {}) => ({ name: "Cosmo", suit: "orange", adopted: 0, last: { fed: 0, patted: 0, played: 0, ...last }, mine: { fed: 0, patted: 0, played: 0 }, xp: 0, level: 1, next_level_xp: 3, cooldown_ms: 600000, ...extra });

test("the app and core agree on suits, the name limit and kinds of care", () => {
  const suits = /SUITS: &\[&str\] = &\[([^\]]+)\]/.exec(core)[1].split(",").map(s => s.trim().replace(/"/g, ""));
  assert.deepEqual(PET_SUITS.map(s => s.id), suits);
  assert.match(core, new RegExp(`MAX_PET_NAME: usize = ${PET_NAME_MAX};`));
  for (const care of PET_CARES) assert.match(core, new RegExp(`Care::\\w+ => "${care.kind}"`));
});

test("the astronaut's mood follows when it was last looked after", () => {
  assert.equal(petMood(pet({}, { adopted: now - 10 * 60 * 1000 }), now).mood, "new");
  assert.equal(petMood(pet(), now).mood, "hungry");
  assert.equal(petMood(pet({ fed: now - HOUR * 2 }), now).mood, "bored");
  assert.equal(petMood(pet({ fed: now - HOUR * 2, played: now - 1000 }), now).mood, "happy");
  assert.equal(petMood(pet({ fed: now - 1000, patted: now - 3 * HOUR }), now).mood, "full");
  assert.equal(petMood(pet({ fed: now - 3 * HOUR, patted: now - 3 * HOUR }), now).mood, "content");
  assert.match(petMood(pet(), now).line, /^Cosmo/);
  assert.equal(petMood(null, now), null);
});

test("care from the far future does not count", () => {
  assert.equal(petMood(pet({ fed: now + 1000 * HOUR, patted: now - HOUR * 2 }), now).mood, "hungry");
});

test("each kind of care waits out its own cooldown", () => {
  const p = pet({}, { mine: { fed: now - 60000, patted: 0, played: 0 } });
  assert.equal(petCareWait(p, "feed", now), 540000);
  assert.equal(petCareWait(p, "pat", now), 0);
  assert.equal(petCareWait(p, "nap", now), 0);
  assert.equal(petCareDone(p, "play"), "Cosmo chases a floating ball");
});

test("level progress fills toward the next level", () => {
  assert.equal(petProgress(pet({}, { xp: 0, level: 1, next_level_xp: 3 })), 0);
  assert.equal(petProgress(pet({}, { xp: 7, level: 2, next_level_xp: 12 })), 4 / 9);
  assert.equal(petProgress(null), 0);
});

test("names are cleaned like core does and the drawing only uses known colours and faces", () => {
  assert.equal(petCleanName(`  ${"a".repeat(40)} `).length, PET_NAME_MAX);
  assert.equal(petCleanName("Co\u0007smo"), "Cosmo");
  const svg = petSvg("purple", "<script>");
  assert.ok(svg.includes("#9b7bff") && svg.includes("mood-content") && !svg.includes("<script>"));
  assert.ok(petSvg("gold", "happy").includes(PET_SUITS[0].color));
});
