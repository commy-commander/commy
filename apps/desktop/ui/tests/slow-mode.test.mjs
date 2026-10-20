import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { SLOW_MODE_OPTIONS, slowModeLabel, slowModeOptionText, slowModeWait, slowModeState, slowModeWaitFromError } = require("../scripts/slow-mode.js");

test("labels read as a short interval and the options climb from off to six hours", () => {
  assert.equal(slowModeLabel(0), "Off");
  assert.equal(slowModeLabel(5), "5s");
  assert.equal(slowModeLabel(120), "2m");
  assert.equal(slowModeLabel(7200), "2h");
  assert.equal(slowModeOptionText(60), "1 minute");
  assert.equal(slowModeOptionText(21600), "6 hours");
  assert.equal(SLOW_MODE_OPTIONS[0], 0);
  assert.equal(SLOW_MODE_OPTIONS[SLOW_MODE_OPTIONS.length - 1], 21600);
});

test("the wait counts down from the last send and never goes negative", () => {
  assert.equal(slowModeWait(1000, 30, 1000), 30);
  assert.equal(slowModeWait(1000, 30, 11_500), 20);
  assert.equal(slowModeWait(1000, 30, 31_000), 0);
  assert.equal(slowModeWait(0, 30, 5000), 0);
  assert.equal(slowModeWait(1000, 0, 2000), 0);
});

test("the composer pill shows only for members in a slow channel and switches to the countdown after a send", () => {
  const channel = { slow_mode: 10 };
  assert.deepEqual(slowModeState({ channel, canBypass: true, sentAt: 0, now: 0 }).shown, false);
  assert.deepEqual(slowModeState({ channel: { slow_mode: 0 }, canBypass: false, sentAt: 0, now: 0 }).shown, false);
  const idle = slowModeState({ channel, canBypass: false, sentAt: 0, now: 5000 });
  assert.deepEqual([idle.shown, idle.waiting, idle.text], [true, 0, "10s"]);
  const waiting = slowModeState({ channel, canBypass: false, sentAt: 1000, now: 4000 });
  assert.deepEqual([waiting.shown, waiting.waiting, waiting.text], [true, 7, "7s"]);
});

test("the wait is read back from the app's refusal so the pill matches the other members' clocks", () => {
  assert.equal(slowModeWaitFromError("slow mode is on here: you can send again in 12s"), 12);
  assert.equal(slowModeWaitFromError("slow mode is on here: you can send again in 2m 5s"), 125);
  assert.equal(slowModeWaitFromError("slow mode is on here: you can send again in 1h 30m"), 5400);
  assert.equal(slowModeWaitFromError("that message is too long"), 0);
});
