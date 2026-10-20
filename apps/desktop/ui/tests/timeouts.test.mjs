import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  TIMEOUT_CHOICES, MAX_TIMEOUT_SECS, timeoutRemainingSecs, timeoutActive, remainingWords, timeoutLeftText,
  timeoutBadgeTitle, timeoutLock, customTimeoutSeconds, canTimeOut, timeoutActionLabel, timeoutSummary,
} = require("../scripts/timeouts.js");

const now = 1_700_000_000_000;

test("the picker offers the fixed lengths in order, all within the longest allowed", () => {
  assert.deepEqual(TIMEOUT_CHOICES.map(c => c.seconds), [60, 300, 600, 3600, 86400, 604800]);
  assert.deepEqual(TIMEOUT_CHOICES.map(c => c.label), ["60 seconds", "5 minutes", "10 minutes", "1 hour", "1 day", "1 week"]);
  assert.ok(TIMEOUT_CHOICES.every(c => c.seconds <= MAX_TIMEOUT_SECS));
});

test("remaining time counts down to zero and rounds up partial seconds", () => {
  assert.equal(timeoutRemainingSecs(now + 90_500, now), 91);
  assert.equal(timeoutRemainingSecs(now + 1, now), 1);
  assert.equal(timeoutRemainingSecs(now, now), 0);
  assert.equal(timeoutRemainingSecs(now - 5000, now), 0);
  assert.equal(timeoutRemainingSecs(null, now), 0);
  assert.equal(timeoutRemainingSecs("nope", now), 0);
  assert.equal(timeoutActive(now + 1000, now), true);
  assert.equal(timeoutActive(now, now), false);
  assert.equal(timeoutActive(undefined, now), false);
});

test("remaining time reads in at most two units", () => {
  assert.equal(remainingWords(0), "0s");
  assert.equal(remainingWords(45), "45s");
  assert.equal(remainingWords(60), "1m 0s");
  assert.equal(remainingWords(252), "4m 12s");
  assert.equal(remainingWords(3600), "1h");
  assert.equal(remainingWords(3600 + 600 + 5), "1h 10m");
  assert.equal(remainingWords(86400 + 3 * 3600), "1d 3h");
  assert.equal(remainingWords(7 * 86400 + 2 * 86400 + 59), "1w 2d");
  assert.equal(timeoutLeftText(now + 252_000, now), "4m 12s");
  assert.equal(timeoutBadgeTitle(now + 252_000, now), "Timed out for another 4m 12s");
});

test("the composer locks while you are timed out and names the time left", () => {
  const server = { my_timeout_until: now + 600_000 };
  const lock = timeoutLock({ view: "server", server, nowMs: now });
  assert.equal(lock.title, "You are timed out");
  assert.match(lock.text, /read and listen/);
  assert.equal(lock.until, now + 600_000);
  assert.equal(timeoutLock({ view: "server", server, nowMs: now + 600_000 }), null, "it lifts on its own");
  assert.equal(timeoutLock({ view: "dm", server, nowMs: now }), null);
  assert.equal(timeoutLock({ view: "server", server: { my_timeout_until: null }, nowMs: now }), null);
  assert.equal(timeoutLock({ view: "server", server: null, nowMs: now }), null);
});

test("custom lengths are whole minutes up to four weeks", () => {
  assert.equal(customTimeoutSeconds("45"), 45 * 60);
  assert.equal(customTimeoutSeconds(" 1 "), 60);
  assert.equal(customTimeoutSeconds("0"), null);
  assert.equal(customTimeoutSeconds("-3"), null);
  assert.equal(customTimeoutSeconds("2.5"), null);
  assert.equal(customTimeoutSeconds(""), null);
  assert.equal(customTimeoutSeconds(null), null);
  assert.equal(customTimeoutSeconds(String(MAX_TIMEOUT_SECS / 60)), MAX_TIMEOUT_SECS);
  assert.equal(customTimeoutSeconds(String(MAX_TIMEOUT_SECS / 60 + 1)), null);
});

test("only moderators with the permission time out members, never the owner or themselves", () => {
  const server = { owner: "own", can_time_out: true, members: [{ id: "own" }, { id: "me" }, { id: "mia" }] };
  assert.equal(canTimeOut(server, { id: "mia" }, "me"), true);
  assert.equal(canTimeOut(server, { id: "own" }, "me"), false);
  assert.equal(canTimeOut(server, { id: "me" }, "me"), false);
  assert.equal(canTimeOut(server, { id: "gone" }, "me"), false);
  assert.equal(canTimeOut({ ...server, can_time_out: false }, { id: "mia" }, "me"), false);
  assert.equal(canTimeOut(null, { id: "mia" }, "me"), false);
});

test("the action flips to removing while a timeout is active", () => {
  assert.equal(timeoutActionLabel({ timeout_until: now + 5000 }, now), "Remove timeout");
  assert.equal(timeoutActionLabel({ timeout_until: now - 5000 }, now), "Time out");
  assert.equal(timeoutActionLabel({}, now), "Time out");
  assert.equal(timeoutActionLabel(null, now), "Time out");
  assert.equal(timeoutSummary("Mia", 600), "Mia is timed out for 10 minutes");
  assert.equal(timeoutSummary("Mia", 45 * 60), "Mia is timed out for 45 minutes");
});
