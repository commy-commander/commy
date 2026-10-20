import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { starfield } = require("../scripts/starfield.js");

test("the same id always paints the same sky", () => {
  assert.equal(starfield("abc").css, starfield("abc").css);
});

test("different ids scatter the stars differently", () => {
  assert.notEqual(starfield("abc").css, starfield("abd").css);
});

test("every sky shares one muted gradient and holds plenty of small, dim stars", () => {
  for (const id of ["abc", "abd", "xyz"]) {
    const { css } = starfield(id);
    assert.match(css, /#12162a, #0b0d17/);
    assert.equal((css.match(/rgba\(242,239,230/g) || []).length, 220);
    const alphas = [...css.matchAll(/rgba\(242,239,230,([\d.]+)\)/g)].map(m => Number(m[1]));
    assert.ok(Math.max(...alphas) <= 0.5);
    const sizes = [...css.matchAll(/,[\d.]+\) ([\d.]+)px, transparent/g)].map(m => Number(m[1]));
    assert.ok(Math.max(...sizes) <= 0.9);
  }
});
