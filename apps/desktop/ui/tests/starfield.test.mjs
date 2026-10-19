import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { starfield, hueGlow } = require("../scripts/starfield.js");

test("the same id always paints the same sky", () => {
  assert.equal(starfield("abc", "71, 87, 214").css, starfield("abc", "71, 87, 214").css);
});

test("different ids paint different skies", () => {
  assert.notEqual(starfield("abc", "1, 2, 3").css, starfield("abd", "1, 2, 3").css);
});

test("the sky is a deep base with a tinted glow and no animation hooks", () => {
  const { css } = starfield("abc", "10, 20, 30");
  assert.match(css, /rgba\(10, 20, 30, 0\.5\)/);
  assert.match(css, /#0b0d17/);
  assert.equal((css.match(/rgba\(242,239,230/g) || []).length, 32);
});

test("hue glows are valid rgb triples", () => {
  for (const h of [0, 90, 180, 270, 359]) {
    const parts = hueGlow(h).split(", ").map(Number);
    assert.equal(parts.length, 3);
    for (const p of parts) assert.ok(p >= 0 && p <= 255);
  }
});
