import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { overlayCornerLabel, OVERLAY_CORNERS, OVERLAY_CHAT_LINES, overlayCorner, overlayUniqueMembers, overlayVisibleMembers, overlayPushLine } = require("../scripts/overlay-view.js");

const members = [{ id: "a" }, { id: "b" }, { id: "c" }];

test("only people talking, or just stopped, are drawn unless everyone is asked for", () => {
  const speaking = new Set(["b"]);
  const lingering = new Set(["c"]);
  assert.deepEqual(overlayVisibleMembers({ members, speaking, lingering, showEveryone: false }).map(m => m.id), ["b", "c"]);
  assert.deepEqual(overlayVisibleMembers({ members, speaking, lingering, showEveryone: true }).map(m => m.id), ["a", "b", "c"]);
  assert.deepEqual(overlayVisibleMembers({ members, speaking: new Set(), lingering: new Set(), showEveryone: false }), []);
  assert.deepEqual(overlayVisibleMembers({ members: undefined, speaking, lingering, showEveryone: false }), []);
});

test("pictures-only mode draws everyone in the channel", () => {
  const speaking = new Set(["b"]);
  assert.deepEqual(overlayVisibleMembers({ members, speaking, lingering: new Set(), showEveryone: false, picturesOnly: true }).map(m => m.id), ["a", "b", "c"]);
});

test("a person listed twice is drawn once", () => {
  const twice = [{ id: "me", name: "Me" }, { id: "b" }, { id: "me", name: "Me" }];
  assert.deepEqual(overlayUniqueMembers(twice).map(m => m.id), ["me", "b"]);
  assert.deepEqual(overlayVisibleMembers({ members: twice, speaking: new Set(["me"]), lingering: new Set(), showEveryone: false }).map(m => m.id), ["me"]);
  assert.deepEqual(overlayUniqueMembers(undefined), []);
});

test("corners say which edges the overlay hugs", () => {
  assert.deepEqual(overlayCorner("bottom-right"), { name: "bottom-right", bottom: true, right: true });
  assert.deepEqual(overlayCorner("top-left"), { name: "top-left", bottom: false, right: false });
  assert.deepEqual(overlayCorner("top-right"), { name: "top-right", bottom: false, right: true });
  assert.deepEqual(overlayCorner("bottom-left"), { name: "bottom-left", bottom: true, right: false });
  assert.equal(overlayCorner("middle").name, "top-left");
  assert.equal(OVERLAY_CORNERS.length, 4);
});

test("chat keeps only the newest few lines", () => {
  let lines = [];
  for (let i = 1; i <= OVERLAY_CHAT_LINES + 3; i++) lines = overlayPushLine(lines, i);
  assert.deepEqual(lines, [4, 5, 6, 7]);
  assert.deepEqual(overlayPushLine(undefined, "x"), ["x"]);
});

test("the note names the corner in words, and unknown corners read as top left", () => {
  assert.equal(overlayCornerLabel("bottom-right"), "Bottom right");
  assert.equal(overlayCornerLabel("top-left"), "Top left");
  assert.equal(overlayCornerLabel("nowhere"), "Top left");
});
