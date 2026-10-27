import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  MAX_ALERT_SOUNDS, ADD_ALERT_SOUND, NO_SOUND,
  isAlertSound, alertSoundId, builtInSoundLabel, alertSoundChoices, resolveAlertSound, joinSoundToPlay,
} = require("../scripts/alert-sounds.js");

test("your own sounds have their own ids that never clash with built-in ones", () => {
  assert.ok(isAlertSound("custom:boop"));
  assert.ok(!isAlertSound("custom:"));
  assert.ok(!isAlertSound("ding"));
  assert.ok(!isAlertSound(undefined));
  assert.equal(alertSoundId("Big Boop!", []), "custom:big_boop");
  assert.equal(alertSoundId("big boop", ["custom:big_boop"]), "custom:big_boop_2");
  assert.equal(alertSoundId("big boop", ["custom:big_boop", "custom:big_boop_2"]), "custom:big_boop_3");
  assert.equal(alertSoundId("!!!", []), "custom:sound");
});

test("the sound lists show built-in sounds, then yours, then add and none", () => {
  const groups = alertSoundChoices(["ding", "ring"], [{ id: "custom:boop", name: "boop" }], true);
  assert.deepEqual(groups.map(g => g.label), ["Built in", "Your sounds", ""]);
  assert.deepEqual(groups[0].options, [{ value: "ding", label: "Ding" }, { value: "ring", label: "Ring" }]);
  assert.deepEqual(groups[1].options, [{ value: "custom:boop", label: "boop" }]);
  assert.deepEqual(groups[2].options.map(o => o.value), [ADD_ALERT_SOUND, NO_SOUND]);
  const full = alertSoundChoices(["ding"], [], false);
  assert.deepEqual(full.map(g => g.label), ["Built in", ""]);
  assert.deepEqual(full[1].options.map(o => o.value), [NO_SOUND]);
  assert.equal(builtInSoundLabel("notification"), "Notification");
  assert.equal(MAX_ALERT_SOUNDS, 10);
});

test("a deleted or missing sound of your own falls back to the built-in one", () => {
  const custom = [{ id: "custom:boop", name: "boop" }];
  assert.equal(resolveAlertSound(undefined, "ring", custom), "ring");
  assert.equal(resolveAlertSound("custom:boop", "ring", custom), "custom:boop");
  assert.equal(resolveAlertSound("custom:gone", "ring", custom), "ring");
  assert.equal(resolveAlertSound("none", "ring", custom), "none");
  assert.equal(resolveAlertSound("pop", "ring", custom), "pop");
});

test("a member's join sound plays only when one person joins and both sides allow it", () => {
  const joinSounds = { a: "blob-a", b: "default:airhorn" };
  const base = { joinSounds, allowed: true, hearJoinSounds: true };
  assert.equal(joinSoundToPlay({ ...base, joined: ["a"] }), "blob-a");
  assert.equal(joinSoundToPlay({ ...base, joined: ["b"] }), "default:airhorn");
  assert.equal(joinSoundToPlay({ ...base, joined: ["c"] }), null);
  assert.equal(joinSoundToPlay({ ...base, joined: ["a", "b"] }), null);
  assert.equal(joinSoundToPlay({ ...base, joined: ["a"], allowed: false }), null);
  assert.equal(joinSoundToPlay({ ...base, joined: ["a"], hearJoinSounds: false }), null);
  assert.equal(joinSoundToPlay({ ...base, joined: ["a"], joinSounds: undefined }), null);
});
