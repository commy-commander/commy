import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const { agoWords, updateStatusText, updateAction } = createRequire(import.meta.url)("../scripts/updates.js");
const now = 1_760_000_000_000;
const ago = secs => now / 1000 - secs;

test("check times read in plain words", () => {
  assert.equal(agoWords(ago(20), now), "just now");
  assert.equal(agoWords(ago(60), now), "1 minute ago");
  assert.equal(agoWords(ago(5 * 60), now), "5 minutes ago");
  assert.equal(agoWords(ago(3 * 3600), now), "3 hours ago");
  assert.equal(agoWords(ago(2 * 86400), now), "2 days ago");
});

test("up to date says when it last looked", () => {
  assert.equal(updateStatusText({ phase: "up_to_date", checked_at: ago(120) }, now), "You have the latest version. Checked 2 minutes ago.");
});

test("a download shows its progress, then asks for a restart", () => {
  assert.equal(updateStatusText({ phase: "downloading", version: "0.2.0", progress: 45 }, now), "Downloading Commy 0.2.0, 45%...");
  assert.equal(updateStatusText({ phase: "ready", version: "0.2.0" }, now), "Commy 0.2.0 is ready. Restart to finish updating.");
  assert.deepEqual(updateAction({ phase: "ready" }), { label: "Restart Now", action: "restart", disabled: false });
});

test("busy states disable the button", () => {
  assert.equal(updateAction({ phase: "checking" }).disabled, true);
  assert.equal(updateAction({ phase: "downloading" }).disabled, true);
  assert.equal(updateAction({ phase: "up_to_date" }).action, "check");
});

test("copies that cannot install point to the download page", () => {
  const status = { phase: "available", reason: "portable", version: "0.2.0", page: "https://example.test/r" };
  assert.match(updateStatusText(status, now), /download the new version yourself/);
  assert.deepEqual(updateAction(status), { label: "Download Page", action: "page", disabled: false });
});

test("development builds never check", () => {
  assert.equal(updateAction({ reason: "dev", phase: "idle" }).disabled, true);
  assert.equal(updateStatusText({ reason: "dev", phase: "idle" }, now), "Development builds do not update themselves.");
});

test("a failed check shows the reason", () => {
  assert.equal(updateStatusText({ phase: "failed", error: "Could not reach the update server.", checked_at: ago(10) }, now), "Could not reach the update server. Checked just now.");
});
