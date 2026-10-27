import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  AVATAR_BORDERS, CARD_EFFECTS, BORDER_ORNAMENTS,
  isAvatarBorder, isCardEffect, ornamentPoints, stringSeed, particleSeeds,
} = require("../scripts/profile-look.js");

const ui = join(import.meta.dirname, "..");
const css = readFileSync(join(ui, "styles", "styles.css"), "utf8");
const user = readFileSync(join(ui, "..", "..", "..", "crates", "commy-core", "src", "user.rs"), "utf8");

test("the app and the UI offer the same borders and effects", () => {
  const listed = name => JSON.parse(`[${new RegExp(`pub const ${name}: \\[&str; \\d+\\] = \\[([^\\]]*)\\]`).exec(user)[1]}]`);
  assert.deepEqual(AVATAR_BORDERS.map(b => b.id), listed("AVATAR_BORDERS"));
  assert.deepEqual(CARD_EFFECTS.map(e => e.id), listed("CARD_EFFECTS"));
  assert.ok(isAvatarBorder("orbit") && !isAvatarBorder("gold") && !isAvatarBorder(undefined));
  assert.ok(isCardEffect("snow") && !isCardEffect("fire"));
});

test("every border and effect has a look, and the moving ones stop for reduced motion", () => {
  for (const { id } of AVATAR_BORDERS) assert.match(css, new RegExp(`\\.ab-${id}[ {.:]`), `${id} border has no look`);
  for (const { id } of CARD_EFFECTS) assert.match(css, new RegExp(`\\.ce-${id}[ {.]`), `${id} effect has no look`);
  assert.match(css, /html\[data-reduced-motion\] \.avatar-border/);
  assert.match(css, /html\[data-reduced-motion\] \.card-effect/);
});

test("ornaments sit evenly on the ring, facing outwards", () => {
  const points = ornamentPoints(3, 180, 360);
  assert.deepEqual(points.map(p => [p.x, p.y]), [[0, 50], [50, 0], [100, 50]]);
  assert.deepEqual(points.map(p => p.angle), [270, 360, 450]);
  assert.deepEqual(ornamentPoints(1, 90, 90), [{ x: 50, y: 100, angle: 180 }]);
  for (const { count, from, to } of Object.values(BORDER_ORNAMENTS)) {
    for (const p of ornamentPoints(count, from, to)) assert.ok(p.x >= 0 && p.x <= 100 && p.y >= 0 && p.y <= 100);
  }
});

test("particles land in the same places for the same person and stay inside the banner", () => {
  assert.equal(stringSeed("abc"), stringSeed("abc"));
  assert.notEqual(stringSeed("abc"), stringSeed("abd"));
  const a = particleSeeds(20, "user-1");
  assert.deepEqual(a, particleSeeds(20, "user-1"));
  assert.notDeepEqual(a, particleSeeds(20, "user-2"));
  for (const p of a) {
    assert.ok(p.x >= 0 && p.x <= 100 && p.y >= 0 && p.y <= 100);
    assert.ok(p.size >= 0.6 && p.size <= 1.4 && p.duration >= 3 && p.duration <= 6 && p.delay >= 0 && p.delay <= 4);
  }
  assert.equal(particleSeeds(0, "x").length, 0);
});
