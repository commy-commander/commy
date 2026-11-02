import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { VOICE_GATE_OPEN, readVoiceGate, voiceGateWait, voiceGateRefusal, sameVoiceGate } = require("../scripts/voice-gate.js");

const catchingUp = { kind: "catching_up", reason: "Getting the latest from the server" };
const gate = readVoiceGate({ enabled: true, waits: { s1: catchingUp } });

test("a server that is still catching up keeps its voice channels closed", () => {
  assert.equal(voiceGateWait(gate, "s1", null), catchingUp);
  assert.equal(voiceGateWait(gate, "s2", null), null);
});

test("turning the setting off opens every voice channel", () => {
  assert.equal(voiceGateWait({ ...gate, enabled: false }, "s1", null), null);
});

test("moving between channels of the server you are already talking in is never held", () => {
  assert.equal(voiceGateWait(gate, "s1", "s1"), null);
  assert.equal(voiceGateWait(gate, "s1", "s2"), catchingUp);
});

test("an app without the check, like an older backend, never blocks voice", () => {
  assert.deepEqual(readVoiceGate(null), VOICE_GATE_OPEN);
  assert.deepEqual(readVoiceGate("nope"), VOICE_GATE_OPEN);
  assert.equal(voiceGateWait(readVoiceGate(undefined), "s1", null), null);
  assert.equal(voiceGateWait(readVoiceGate({ enabled: true }), "s1", null), null);
});

test("the refusal says why voice is closed", () => {
  assert.equal(voiceGateRefusal(catchingUp), "Voice opens once you are connected. Getting the latest from the server.");
});

test("the channel list repaints only when a server's wait changes", () => {
  assert.ok(sameVoiceGate(gate, readVoiceGate({ enabled: true, waits: { s1: { ...catchingUp } } })));
  assert.ok(!sameVoiceGate(gate, readVoiceGate({ enabled: true, waits: { s1: { kind: "sync_lost", reason: "Reconnecting to the server" } } })));
  assert.ok(!sameVoiceGate(gate, readVoiceGate({ enabled: true, waits: {} })));
  assert.ok(!sameVoiceGate(gate, readVoiceGate({ enabled: false, waits: { s1: catchingUp } })));
});
