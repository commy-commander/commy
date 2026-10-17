import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { personTint } = require("../scripts/tint.js");

test("a person always gets the same tint, and different people usually differ", () => {
  assert.equal(personTint("m1"), personTint("m1"));
  assert.match(personTint("m1"), /^hsl\(\d{1,3} 38% 44%\)$/);
  assert.notEqual(personTint("ferris"), personTint("nova"));
  assert.match(personTint(undefined), /^hsl\(/);
});
