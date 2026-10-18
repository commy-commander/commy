import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { membersCollapsed } = require("../scripts/layout.js");

test("the people panel starts collapsed on narrow windows and open on wide ones", () => {
  assert.equal(membersCollapsed(null, 1000), true);
  assert.equal(membersCollapsed(null, 1600), false);
});

test("a choice made with the toggle wins over the window width", () => {
  assert.equal(membersCollapsed("0", 1000), false);
  assert.equal(membersCollapsed("1", 2000), true);
});
