import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync, statSync } from "node:fs";

const require = createRequire(import.meta.url);
const {
  MAX_ALERT_SOUNDS, ADD_ALERT_SOUND, NO_SOUND,
  isAlertSound, alertSoundId, builtInSoundLabel, alertSoundChoices, resolveAlertSound, builtInSoundFile, joinSoundToPlay,
  voiceRosterChange, monoSamples, soundLevel,
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

test("a voice roster change names who joined and whether anyone left", () => {
  assert.deepEqual(voiceRosterChange(new Set(["a"]), new Set(["a", "b"])), { joined: ["b"], left: false });
  assert.deepEqual(voiceRosterChange(new Set(["a", "b"]), new Set(["b"])), { joined: [], left: true });
  assert.deepEqual(voiceRosterChange(new Set(["a"]), new Set(["c"])), { joined: ["c"], left: true });
  assert.deepEqual(voiceRosterChange(new Set(), new Set()), { joined: [], left: false });
});

test("a decoded sound becomes one channel for the chosen output", () => {
  const left = Float32Array.of(0.5, 1, -1);
  const right = Float32Array.of(0.5, 0, 1, 0.25);
  assert.deepEqual(Array.from(monoSamples([left, right])), [0.5, 0.5, 0]);
  assert.deepEqual(Array.from(monoSamples([left])), [0.5, 1, -1]);
  assert.equal(monoSamples([]).length, 0);
});

test("sound levels stay between silent and full", () => {
  assert.equal(soundLevel(0.4), 0.4);
  assert.equal(soundLevel(3), 1);
  assert.equal(soundLevel(-1), 0);
  assert.equal(soundLevel(undefined), 1);
});

test("the new look has its own sounds for a new message and a direct message, and every other sound stays", () => {
  const app = readFileSync(new URL("../scripts/app.js", import.meta.url), "utf8").replace(/\r\n/g, "\n");
  const files = { notification: "sounds/notification.mp3", ding: "sounds/ding.mp3", ring: "sounds/ring.wav" };
  const studio = { notification: "sounds/studio/notification.mp3", ding: "sounds/studio/ding.mp3" };
  assert.equal(builtInSoundFile("notification", files, studio, true), "sounds/studio/notification.mp3");
  assert.equal(builtInSoundFile("ding", files, studio, true), "sounds/studio/ding.mp3");
  assert.equal(builtInSoundFile("ring", files, studio, true), "sounds/ring.wav");
  assert.equal(builtInSoundFile("notification", files, studio, false), "sounds/notification.mp3");
  assert.equal(builtInSoundFile("ding", files, studio, false), "sounds/ding.mp3");
  assert.equal(builtInSoundFile("nothing", files, studio, true), null);
  assert.match(app, /const STUDIO_SOUND_FILES = \{ notification: "sounds\/studio\/notification\.mp3", ding: "sounds\/studio\/ding\.mp3" \};/);
  assert.match(app, /return builtInSoundFile\(name, SOUND_FILES, STUDIO_SOUND_FILES, studioLook\(\)\);/);
  for (const name of ["notification", "ding"]) assert.ok(statSync(new URL(`../sounds/studio/${name}.mp3`, import.meta.url)).size > 10000);
});
