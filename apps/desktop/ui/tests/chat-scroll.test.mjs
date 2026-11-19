import "./i18n-global.mjs";
import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { scrollDownShown, centeredScrollTop, landingScrollTop, distanceFromEnd, atListEnd, redrawnScrollTop, landedMessageIds } = require("../scripts/chat-scroll.js");

test("the scroll down button stays hidden near the newest messages", () => {
  assert.equal(scrollDownShown(2000, 1500, 500), false);
  assert.equal(scrollDownShown(2000, 1200, 500), false);
});

test("the scroll down button shows once the reader is well above the newest messages", () => {
  assert.equal(scrollDownShown(3000, 0, 500), true);
});

test("an empty or hidden message list never shows the button", () => {
  assert.equal(scrollDownShown(0, 0, 0), false);
});

test("a jumped-to message lands in the middle of the message list", () => {
  assert.equal(centeredScrollTop(1000, 100, 600, 5000), 750);
});

test("a jump near either end of the history stops at the edge of the list", () => {
  assert.equal(centeredScrollTop(40, 100, 600, 5000), 0);
  assert.equal(centeredScrollTop(4900, 100, 600, 5000), 4400);
});

test("a short history never scrolls", () => {
  assert.equal(centeredScrollTop(200, 100, 600, 500), 0);
});

test("opening a channel with nothing new lands on the newest message", () => {
  assert.equal(landingScrollTop(2000, 500, null), 1500);
});

test("opening a channel with new messages lands on the NEW line when it is above the newest screen", () => {
  assert.equal(landingScrollTop(3000, 500, 1200), 1192);
});

test("a NEW line already on the newest screen keeps the view at the bottom", () => {
  assert.equal(landingScrollTop(2000, 500, 1800), 1500);
});

test("a short channel never scrolls", () => {
  assert.equal(landingScrollTop(400, 500, 100), 0);
});

test("only a list scrolled all the way down counts as sitting at the newest message", () => {
  assert.equal(atListEnd(2000, 1500, 500), true);
  assert.equal(atListEnd(2000, 1498.5, 500), true);
  assert.equal(atListEnd(2000, 1460, 500), false);
});

test("the distance from the newest message never goes below zero", () => {
  assert.equal(distanceFromEnd(2000, 1300, 500), 200);
  assert.equal(distanceFromEnd(400, 0, 500), 0);
});

test("a redraw leaves a reader who scrolled up a little exactly where they were", () => {
  assert.equal(redrawnScrollTop({ scrollTop: 1460, fromEnd: 40, pinned: false, newer: false }, 2000, 500), 1460);
  assert.equal(redrawnScrollTop({ scrollTop: 1300, fromEnd: 200, pinned: false, newer: false }, 2000, 500), 1300);
});

test("a redraw keeps a reader at the newest message there as the list grows", () => {
  assert.equal(redrawnScrollTop({ scrollTop: 1500, fromEnd: 0, pinned: true, newer: false }, 2300, 500), 1800);
});

test("a new message pulls a reader who is just above the newest message down to it", () => {
  assert.equal(redrawnScrollTop({ scrollTop: 1460, fromEnd: 40, pinned: false, newer: true }, 2100, 500), 1600);
});

test("a new message leaves a reader further up the history in place", () => {
  assert.equal(redrawnScrollTop({ scrollTop: 900, fromEnd: 600, pinned: false, newer: true }, 2100, 500), 900);
});

test("a redraw that shortens the list stops at its end", () => {
  assert.equal(redrawnScrollTop({ scrollTop: 1460, fromEnd: 40, pinned: false, newer: false }, 1700, 500), 1200);
});

test("only messages that arrive after the newest one already shown land", () => {
  const ids = list => list.map(id => ({ id }));
  assert.deepEqual([...landedMessageIds(ids(["a", "b"]), ids(["a", "b", "c"]))], ["c"]);
  assert.deepEqual([...landedMessageIds(ids(["c", "d"]), ids(["a", "b", "c", "d"]))], []);
  assert.deepEqual([...landedMessageIds(ids(["a", "b"]), ids(["a", "b"]))], []);
  assert.deepEqual([...landedMessageIds([], ids(["a"]))], []);
  assert.deepEqual([...landedMessageIds(ids(["x"]), ids(["a", "b"]))], []);
  assert.deepEqual([...landedMessageIds(ids(["a"]), ids(["a", "b", "c", "d", "e", "f", "g"]))], []);
});
