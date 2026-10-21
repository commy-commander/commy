import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  VOICE_STATUS_MAX_CHARS,
  PRIORITY_DUCK_GAIN,
  cleanVoiceStatus,
  voiceStatusProblem,
  canSetVoiceStatus,
  prioritySpeakersIn,
  priorityTalking,
  attenuationGain,
  attenuatedUsers,
} = require("../scripts/voice-status.js");

const members = [
  { id: "m1", name: "ferris", priority_speaker: true },
  { id: "m2", name: "nova", priority_speaker: false },
  { id: "m3", name: "sleepy" },
];
const room = { kind: "voice", voice_user_ids: ["m1", "m2", "m3"] };

test("statuses are tidied to one line of at most eighty characters", () => {
  assert.equal(VOICE_STATUS_MAX_CHARS, 80);
  assert.equal(cleanVoiceStatus("  Ranked \n\t grind  "), "Ranked grind");
  assert.equal(cleanVoiceStatus("Ran\u0000ked"), "Ranked");
  assert.equal(cleanVoiceStatus(""), "");
  assert.equal(cleanVoiceStatus(null), "");
  assert.equal([...cleanVoiceStatus("a".repeat(100))].length, 80);
  assert.equal(cleanVoiceStatus(`${"b".repeat(79)} tail`), "b".repeat(79));
  assert.equal(voiceStatusProblem("Ranked grind"), null);
  assert.equal(voiceStatusProblem(" ".repeat(90)), null);
  assert.match(voiceStatusProblem("x".repeat(81)), /80 characters/);
});

test("the backend's say on who may set a status wins, with a sensible fallback", () => {
  assert.equal(canSetVoiceStatus({ channel: { kind: "voice", can_set_voice_status: true } }), true);
  assert.equal(canSetVoiceStatus({ channel: { kind: "voice", can_set_voice_status: false }, server: { can_manage_channels: true } }), false);
  assert.equal(canSetVoiceStatus({ channel: { kind: "text", can_set_voice_status: true } }), false);
  assert.equal(canSetVoiceStatus({ channel: room, server: { can_manage_channels: true } }), true);
  assert.equal(canSetVoiceStatus({ channel: room, server: {}, me: "m2" }), true);
  assert.equal(canSetVoiceStatus({ channel: room, server: {}, me: "me" }), false);
  assert.equal(canSetVoiceStatus({ channel: null }), false);
});

test("priority speakers are the members in the room who hold the permission", () => {
  assert.deepEqual([...prioritySpeakersIn(room, members)], ["m1"]);
  assert.deepEqual([...prioritySpeakersIn({ voice_user_ids: ["m2"] }, members)], []);
  assert.deepEqual([...prioritySpeakersIn(null, members)], []);
  assert.equal(priorityTalking(new Set(["m1"]), new Set(["m1", "m2"])), true);
  assert.equal(priorityTalking(new Set(["m1"]), new Set(["m2"])), false);
  assert.equal(priorityTalking(new Set(), new Set(["m1"])), false);
});

test("a talking priority speaker lowers everyone else to thirty percent", () => {
  assert.equal(PRIORITY_DUCK_GAIN, 0.3);
  assert.equal(attenuationGain(1, { ducked: true }), 0.3);
  assert.ok(Math.abs(attenuationGain(0.5, { ducked: true }) - 0.15) < 1e-9);
  assert.equal(attenuationGain(1, { ducked: true, isPriority: true }), 1);
  assert.equal(attenuationGain(1.5, { ducked: false }), 1.5);
  assert.equal(attenuationGain(undefined, { ducked: true }), 0.3);
  assert.deepEqual([...attenuatedUsers(room, members, new Set(["m1"]))], ["m2", "m3"]);
  assert.deepEqual([...attenuatedUsers(room, members, new Set(["m2"]))], []);
  assert.deepEqual([...attenuatedUsers(room, members, new Set())], []);
});
