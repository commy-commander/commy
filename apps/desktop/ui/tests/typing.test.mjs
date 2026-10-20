import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { TYPING_TTL_MS, TYPING_SEND_EVERY_MS, TypingTracker, TypingSender, typingLine } = require("../scripts/typing.js");

test("typers expire after the time to live and clear when their message lands", () => {
  const t = new TypingTracker();
  t.start("c1", "a", 1000);
  t.start("c1", "b", 2000);
  t.start("c2", "c", 1000);
  assert.deepEqual(t.typers("c1", 3000), ["a", "b"]);
  assert.deepEqual(t.typers("c1", 1000 + TYPING_TTL_MS), ["b"]);
  t.stop("c1", "b");
  assert.deepEqual(t.typers("c1", 3000), []);
  assert.equal(t.anyone(1000 + TYPING_TTL_MS - 1), true);
  assert.equal(t.anyone(2000 + TYPING_TTL_MS), false);
  assert.deepEqual(t.typers("missing"), []);
  t.start("", "x");
  assert.equal(t.anyone(0), false);
});

test("a fresh typing event extends the time to live", () => {
  const t = new TypingTracker();
  t.start("c", "a", 0);
  t.start("c", "a", 5000);
  assert.deepEqual(t.typers("c", 5000 + TYPING_TTL_MS - 1), ["a"]);
});

test("the line reads naturally for one to many people", () => {
  assert.equal(typingLine([]), "");
  assert.equal(typingLine(["ferris"]), "ferris is typing");
  assert.equal(typingLine(["ferris", "nova"]), "ferris and nova are typing");
  assert.equal(typingLine(["a", "b", "c"]), "a, b and c are typing");
  assert.equal(typingLine(["a", "b", "c", "d"]), "a, b, c and 1 others are typing".replace("1 others", "1 others"));
  assert.equal(typingLine(["a", "b", "c", "d", "e", "f"]), "Several people are typing");
  assert.equal(typingLine(["a", null, "b"]), "a and b are typing");
});

test("the sender throttles to one event per interval per channel and resets after a send", () => {
  const sent = [];
  const s = new TypingSender(c => sent.push(c));
  assert.equal(s.typed("c", "h", 0), true);
  assert.equal(s.typed("c", "he", 1000), false);
  assert.equal(s.typed("c", "hel", TYPING_SEND_EVERY_MS), true);
  assert.equal(s.typed("d", "x", TYPING_SEND_EVERY_MS + 1), true);
  assert.equal(s.typed("d", "   ", TYPING_SEND_EVERY_MS * 3), false);
  assert.equal(s.typed(null, "x", TYPING_SEND_EVERY_MS * 3), false);
  s.sent();
  assert.equal(s.typed("d", "x", TYPING_SEND_EVERY_MS + 2), true);
  assert.deepEqual(sent, ["c", "c", "d", "d"]);
});
