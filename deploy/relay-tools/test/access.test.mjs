import assert from "node:assert/strict";
import { test } from "node:test";
import { accessMode, Answers, authUrls, combineAnswers, DEFAULT_AUTH_URL, emptyTally, letIn, LINKED_KEEP_MS, loadAccessConfig, STRANGER_KEEP_MS, tally, tallyReport } from "../lib.mjs";

const A = "aa".repeat(32);
const B = "bb".repeat(32);

test("only an explicit enforce turns refusals on", () => {
  assert.equal(accessMode("enforce"), "enforce");
  assert.equal(accessMode(" Enforce "), "enforce");
  assert.equal(accessMode(undefined), "watch");
  assert.equal(accessMode("off"), "watch");
});

test("watching lets everyone in, enforcing refuses only endpoints auth says are not linked", () => {
  assert.equal(letIn("watch", false), true);
  assert.equal(letIn("enforce", true), true);
  assert.equal(letIn("enforce", false), false);
  assert.equal(letIn("enforce", undefined), true, "an auth outage must not cut voice off");
});

test("linked answers are kept for minutes, refusals only for seconds", () => {
  const answers = new Answers();
  answers.set(A, true, 0);
  answers.set(B, false, 0);
  assert.equal(answers.get(A, LINKED_KEEP_MS - 1), true);
  assert.equal(answers.get(B, STRANGER_KEEP_MS - 1), false);
  assert.equal(answers.get(B, STRANGER_KEEP_MS), undefined, "a device that just signed in gets on at its next retry");
  assert.equal(answers.get(A, LINKED_KEEP_MS), undefined);
});

test("the answer cache stays bounded and drops the oldest first", () => {
  const answers = new Answers(2);
  answers.set("1", true, 0);
  answers.set("2", true, 0);
  answers.set("3", true, 0);
  assert.equal(answers.get("1", 1), undefined);
  assert.equal(answers.get("3", 1), true);
});

test("the daily tally counts each endpoint once and starts over each day", () => {
  const day = new Date("2026-11-15T10:00:00Z");
  let t = emptyTally(day);
  t = tally(t, day, A, { linked: true, allowed: true });
  t = tally(t, day, A, { linked: true, allowed: true });
  t = tally(t, day, B, { linked: false, allowed: false });
  t = tally(t, day, B, { linked: false, allowed: false });
  t = tally(t, day, A, { linked: undefined, allowed: true });
  assert.deepEqual(tallyReport(t, "enforce"), { mode: "enforce", day: "2026-11-15", endpoints: 2, strangers: 1, refused: 2, unchecked: 1 });
  const next = tally(t, new Date("2026-11-16T00:00:01Z"), A, { linked: true, allowed: true });
  assert.deepEqual(tallyReport(next, "enforce"), { mode: "enforce", day: "2026-11-16", endpoints: 1, strangers: 0, refused: 0, unchecked: 0 });
});

test("config defaults point at the auth service on the same network", () => {
  const cfg = loadAccessConfig({});
  assert.equal(cfg.mode, "watch");
  assert.deepEqual(cfg.authUrls, [DEFAULT_AUTH_URL]);
  assert.equal(cfg.port, 3342);
});

test("a test relay can ask several account servers", () => {
  assert.deepEqual(authUrls(" http://commy-staging-auth:3001/internal/relay-access, http://commy-dev-auth:3001/internal/relay-access ,nope"), [
    "http://commy-staging-auth:3001/internal/relay-access",
    "http://commy-dev-auth:3001/internal/relay-access",
  ]);
});

test("one account server knowing the device is enough, and a silent one never refuses", () => {
  assert.equal(combineAnswers([false, true]), true);
  assert.equal(combineAnswers([false, false]), false);
  assert.equal(combineAnswers([false, undefined]), undefined);
  assert.equal(combineAnswers([]), undefined);
});
