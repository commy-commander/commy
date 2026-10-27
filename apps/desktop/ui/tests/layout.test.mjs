import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { membersCollapsed, clampSidebarWidth, pickConnLink } = require("../scripts/layout.js");

test("the people panel starts collapsed on narrow windows and open on wide ones", () => {
  assert.equal(membersCollapsed(null, 1000), true);
  assert.equal(membersCollapsed(null, 1600), false);
});

test("a choice made with the toggle wins over the window width", () => {
  assert.equal(membersCollapsed("0", 1000), false);
  assert.equal(membersCollapsed("1", 2000), true);
});

test("members always give way to the chat on small windows", () => {
  assert.equal(membersCollapsed("0", 800), true);
  assert.equal(membersCollapsed("0", 1000), false);
});

test("sidebar width stays within sensible limits", () => {
  assert.equal(clampSidebarWidth(100, 1400), 180);
  assert.equal(clampSidebarWidth(300, 1400), 300);
  assert.equal(clampSidebarWidth(900, 1400), 420);
  assert.equal(clampSidebarWidth(400, 800), 360);
});

test("connection panel keeps the chosen person and falls back to the first", () => {
  const links = [{ user: "a" }, { user: "b" }];
  assert.equal(pickConnLink(links, "b").user, "b");
  assert.equal(pickConnLink(links, "gone").user, "a");
  assert.equal(pickConnLink(links, null).user, "a");
  assert.equal(pickConnLink([], "a"), null);
});

test("voice rings fold everyone past the fourth into one more-people ring", () => {
  const { voiceRingSplit } = require("../scripts/layout.js");
  assert.deepEqual(voiceRingSplit(3), { shown: 3, more: 0 });
  assert.deepEqual(voiceRingSplit(4), { shown: 4, more: 0 });
  assert.deepEqual(voiceRingSplit(5), { shown: 3, more: 2 });
  assert.deepEqual(voiceRingSplit(9), { shown: 3, more: 6 });
});
