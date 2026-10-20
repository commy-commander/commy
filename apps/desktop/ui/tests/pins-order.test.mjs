import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { channelPinKey, splitPinned, orderPinned, isPinned, togglePinned, pinnedChannelIds } = require("../scripts/pins-order.js");

const threads = [
  { user: "a", name: "alpha" },
  { user: "b", name: "bravo" },
  { user: "c", name: "charlie" },
  { user: "d", name: "delta" },
];

test("pinned items float to the top and both groups keep their original relative order", () => {
  const ordered = orderPinned(threads, ["d", "b"], t => t.user);
  assert.deepEqual(ordered.map(t => t.user), ["b", "d", "a", "c"]);
  const { pinned, rest } = splitPinned(threads, new Set(["c"]), t => t.user);
  assert.deepEqual(pinned.map(t => t.user), ["c"]);
  assert.deepEqual(rest.map(t => t.user), ["a", "b", "d"]);
});

test("unknown pins are ignored and empty input stays empty", () => {
  assert.deepEqual(orderPinned(threads, ["zz"], t => t.user), threads);
  assert.deepEqual(orderPinned(threads, null, t => t.user), threads);
  assert.deepEqual(splitPinned(undefined, ["a"]), { pinned: [], rest: [] });
  assert.deepEqual(orderPinned([{ id: 1 }, { id: 2 }], [2]), [{ id: 2 }, { id: 1 }]);
});

test("toggling pins is idempotent and never mutates the list it was given", () => {
  const pins = ["a"];
  assert.deepEqual(togglePinned(pins, "b", true), ["a", "b"]);
  assert.deepEqual(togglePinned(pins, "a", true), ["a"]);
  assert.deepEqual(togglePinned(pins, "a", false), []);
  assert.deepEqual(togglePinned(pins, "zz", false), ["a"]);
  assert.deepEqual(togglePinned(pins, "a"), []);
  assert.deepEqual(togglePinned(pins, "b"), ["a", "b"]);
  assert.deepEqual(pins, ["a"]);
  assert.equal(isPinned(pins, "a"), true);
  assert.equal(isPinned(new Set(pins), "b"), false);
  assert.equal(isPinned(undefined, "a"), false);
});

test("channel pins are picked per server", () => {
  const pins = { dms: ["a"], channels: [{ server: "s1", channel: "c1" }, { server: "s2", channel: "c2" }, { server: "s1", channel: "c3" }] };
  assert.deepEqual(pinnedChannelIds(pins, "s1"), ["c1", "c3"]);
  assert.deepEqual(pinnedChannelIds(pins, "s3"), []);
  assert.deepEqual(pinnedChannelIds(null, "s1"), []);
  assert.equal(channelPinKey("s1", "c1"), "s1:c1");
});
