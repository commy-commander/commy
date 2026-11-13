import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { CATEGORY_BASE_POSITION, reorderKeys, looseStayFirst, categoryMoves, sectionChannelMoves, dropLandsAfter } = require("../scripts/channel-order.js");

const keys = ["text", "voice", "a", "b", "c"];

test("a group lands before or after the one it is dropped on", () => {
  assert.deepEqual(reorderKeys(keys, "c", "a", false), ["text", "voice", "c", "a", "b"]);
  assert.deepEqual(reorderKeys(keys, "a", "c", true), ["text", "voice", "b", "c", "a"]);
  assert.deepEqual(reorderKeys(keys, "voice", "text", false), ["voice", "text", "a", "b", "c"]);
  assert.deepEqual(reorderKeys(keys, "text", "c", true), ["voice", "a", "b", "c", "text"]);
});

test("without a known target it goes to the top or the bottom", () => {
  assert.deepEqual(reorderKeys(keys, "b", null, false), ["b", "text", "voice", "a", "c"]);
  assert.deepEqual(reorderKeys(keys, "a", "gone", true), ["text", "voice", "b", "c", "a"]);
});

test("a drop that changes nothing gives nothing", () => {
  assert.equal(reorderKeys(keys, "a", "a", false), null);
  assert.equal(reorderKeys(keys, "a", "b", false), null);
  assert.equal(reorderKeys(keys, "b", "a", true), null);
  assert.equal(reorderKeys(keys, "gone", "a", false), null);
});

test("the built-in groups only need to become categories once they leave the top", () => {
  assert.equal(looseStayFirst(["text", "voice", "c", "a", "b"], ["text", "voice"]), true);
  assert.equal(looseStayFirst(["voice", "text", "a", "b", "c"], ["text", "voice"]), false);
  assert.equal(looseStayFirst(["text", "a", "voice", "b", "c"], ["text", "voice"]), false);
  assert.equal(looseStayFirst(["a", "b"], []), true);
});

test("moves give categories fresh top level positions and channels their group", () => {
  assert.deepEqual(categoryMoves(["b", "a"], k => `id-${k}`), [
    { channel: "id-b", parent: null, position: CATEGORY_BASE_POSITION },
    { channel: "id-a", parent: null, position: CATEGORY_BASE_POSITION + 1 },
  ]);
  assert.deepEqual(sectionChannelMoves([{ id: "x" }, { id: "y" }], "cat"), [
    { channel: "x", parent: "cat", position: 0 },
    { channel: "y", parent: "cat", position: 1 },
  ]);
});

test("the lower half of a header means after it", () => {
  const rect = { top: 100, height: 20 };
  assert.equal(dropLandsAfter(105, rect), false);
  assert.equal(dropLandsAfter(115, rect), true);
});
