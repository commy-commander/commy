import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { scrollDownShown, centeredScrollTop, landingScrollTop } = require("../scripts/chat-scroll.js");

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
