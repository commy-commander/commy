import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const { elapsedClock, elapsedWords } = createRequire(import.meta.url)("../scripts/timers.js");
const now = 1_760_000_000_000;
const ago = secs => now / 1000 - secs;

test("clock counts minutes and seconds, then hours", () => {
  assert.equal(elapsedClock(ago(0), now), "0:00");
  assert.equal(elapsedClock(ago(754), now), "12:34");
  assert.equal(elapsedClock(ago(3723), now), "1:02:03");
});

test("clock never runs backwards from a future start", () => {
  assert.equal(elapsedClock(ago(-30), now), "0:00");
});

test("words round down to hours and minutes", () => {
  assert.equal(elapsedWords(ago(30), now), "under a minute");
  assert.equal(elapsedWords(ago(12 * 60 + 59), now), "12m");
  assert.equal(elapsedWords(ago(4332), now), "1h 12m");
});
