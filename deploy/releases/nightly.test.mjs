import { test } from "node:test";
import assert from "node:assert/strict";
import { buildPlan, isNightlyVersion, nightliesToPrune, nightlyVersion, planFromEnv } from "./nightly.mjs";

const date = new Date("2026-11-18T04:17:00Z");
const monday = new Date("2026-11-16T04:17:00Z");

test("a nightly builds the version after the newest stable", () => {
  assert.equal(nightlyVersion({ appVersion: "0.1.0", stableVersion: "1.0.0", date, run: 42 }), "1.0.1-nightly.20261118.42");
});

test("a nightly follows the app version when main is already ahead of stable", () => {
  assert.equal(nightlyVersion({ appVersion: "1.1.0", stableVersion: "1.0.0", date, run: 42 }), "1.1.0-nightly.20261118.42");
});

test("a nightly still gets a version when the stable feed cannot be read", () => {
  assert.equal(nightlyVersion({ appVersion: "1.0.0", stableVersion: undefined, date, run: 7 }), "1.0.0-nightly.20261118.7");
});

test("a nightly needs a real run number", () => {
  for (const run of [undefined, "", "07", "x"]) assert.throws(() => nightlyVersion({ appVersion: "1.0.0", stableVersion: "1.0.0", date, run }));
});

test("nightly versions are recognised by their shape", () => {
  assert.equal(isNightlyVersion("1.0.1-nightly.20261118.42"), true);
  for (const version of ["1.0.1", "1.0.1-rc.1", "1.0.1-nightly", "nightly/../x", undefined]) assert.equal(isNightlyVersion(version), false);
});

test("pruning keeps the newest nightlies by day and run", () => {
  const versions = ["1.0.1-nightly.20261118.42", "1.0.1-nightly.20261116.40", "1.0.2-nightly.20261119.43", "1.0.1-nightly.20261117.41"];
  assert.deepEqual(nightliesToPrune(versions, 2), ["1.0.1-nightly.20261117.41", "1.0.1-nightly.20261116.40"]);
  assert.equal(nightliesToPrune(versions, 0).includes("1.0.2-nightly.20261119.43"), false);
  assert.deepEqual(nightliesToPrune(["1.0.1-nightly.20261118.42", "stable"], 1), []);
});

const main = { event: "schedule", ref: "refs/heads/main", enabled: true, asked: false, sha: "new", lastCommit: "old", date };

test("a scheduled run on main builds a nightly when main changed", () => {
  assert.deepEqual(buildPlan(main), { build: true, nightly: true, reason: "main changed since the last nightly" });
});

test("a scheduled run skips the nightly when main has not changed", () => {
  assert.equal(buildPlan({ ...main, lastCommit: "new" }).build, false);
});

test("with nightlies off the schedule only warms the caches on Mondays and Thursdays", () => {
  assert.deepEqual(buildPlan({ ...main, enabled: false }), { build: false, nightly: false, reason: "nightly builds are off" });
  assert.deepEqual(buildPlan({ ...main, enabled: false, date: monday }), { build: true, nightly: false, reason: "keeping the build caches warm" });
});

test("tag pushes and plain dispatches build as before", () => {
  assert.deepEqual(buildPlan({ ...main, event: "push", ref: "refs/tags/v1.0.0" }), { build: true, nightly: false, reason: "a push build" });
  assert.equal(buildPlan({ ...main, event: "workflow_dispatch", enabled: false }).nightly, false);
});

test("an asked-for nightly runs only from main with nightlies on", () => {
  assert.equal(buildPlan({ ...main, event: "workflow_dispatch", asked: true, lastCommit: "new" }).nightly, true);
  assert.throws(() => buildPlan({ ...main, event: "workflow_dispatch", asked: true, ref: "refs/heads/feat/x" }), /only from main/);
  assert.throws(() => buildPlan({ ...main, event: "workflow_dispatch", asked: true, enabled: false }), /only from main/);
});

test("the plan reads the feeds and the app config it needs", async () => {
  const feeds = {
    "https://releases.commy.chat/nightly/latest.json": { version: "1.0.1-nightly.20261117.41", commit: "old" },
    "https://releases.commy.chat/stable/latest.json": { version: "1.0.0" },
  };
  const env = { GITHUB_EVENT_NAME: "schedule", GITHUB_REF: "refs/heads/main", GITHUB_SHA: "new", GITHUB_RUN_NUMBER: "42", NIGHTLY_BUILDS: "true" };
  const plan = await planFromEnv({ env, date, fetchJson: async (url) => feeds[url] ?? null, readText: async () => '{"version":"0.1.0"}' });
  assert.equal(plan.version, "1.0.1-nightly.20261118.42");
  const unchanged = await planFromEnv({ env: { ...env, GITHUB_SHA: "old" }, date, fetchJson: async (url) => feeds[url] ?? null, readText: async () => "{}" });
  assert.deepEqual(unchanged, { build: false, nightly: false, reason: "main has not changed since the last nightly", version: "" });
});
