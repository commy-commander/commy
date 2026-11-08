import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { scrollDownShown } = require("../scripts/chat-scroll.js");

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
