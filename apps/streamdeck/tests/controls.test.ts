import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { actionId, CONTROL_KINDS, dialControl, keyLook, pressControl, releaseControl } from "../src/controls.ts";
import type { DeckStatus } from "../src/protocol.ts";

const status: DeckStatus = {
  muted: true,
  deafened: false,
  push_to_talk: false,
  voice: { server: "s1", channel: "c1" },
  camera: false,
  sharing: true,
  output_volume: 0.8,
  input_gain: 1.2,
  input_device: null,
  output_device: "Desk speakers",
  user_volumes: [{ user: "u1", volume: 0.5 }],
  unread: 4,
  servers: [{ id: "s1", members: 40, online: 12, in_voice: 3 }],
};

test("every kind has an action in the manifest", () => {
  const manifest = JSON.parse(readFileSync(new URL("../chat.commy.soundboard.sdPlugin/manifest.json", import.meta.url), "utf8"));
  const uuids = manifest.Actions.map((a: { UUID: string }) => a.UUID);
  for (const kind of CONTROL_KINDS) assert.ok(uuids.includes(actionId(kind)), kind);
});

test("toggles and holds send the matching control", () => {
  assert.deepEqual(pressControl("mute", {}), { action: "toggle_mute" });
  assert.deepEqual(pressControl("push-to-talk", {}), { action: "talk", down: true });
  assert.deepEqual(releaseControl("push-to-talk"), { action: "talk", down: false });
  assert.deepEqual(releaseControl("push-to-mute"), { action: "hold_mute", down: false });
  assert.equal(releaseControl("mute"), null);
  assert.deepEqual(pressControl("notifications", {}), { action: "open_app" });
});

test("keys that need a choice do nothing until it is made", () => {
  assert.equal(pressControl("voice-channel", { server: "s1" }), null);
  assert.deepEqual(pressControl("voice-channel", { server: "s1", channel: "c1" }), { action: "join_voice", server: "s1", channel: "c1" });
  assert.equal(pressControl("user-volume", {}), null);
  assert.equal(pressControl("audio-device", {}), null);
  assert.deepEqual(pressControl("audio-device", { target: "input", device: "" }), { action: "device", target: "input", device: "" });
});

test("volume keys step, set and turn by dial", () => {
  assert.deepEqual(pressControl("volume", {}), { action: "volume", target: "output", change: { by: 0.1 } });
  assert.deepEqual(pressControl("volume", { target: "input", mode: "down", amount: 25 }), { action: "volume", target: "input", change: { by: -0.25 } });
  assert.deepEqual(pressControl("volume", { mode: "set", amount: 500 }), { action: "volume", target: "output", change: { to: 2 } });
  assert.deepEqual(dialControl("volume", {}, -3), { action: "volume", target: "output", change: { by: -0.06 } });
  assert.deepEqual(dialControl("user-volume", { user: "u1" }, 1), { action: "user_volume", user: "u1", change: { by: 0.02 } });
  assert.equal(dialControl("mute", {}, 1), null);
});

test("keys show what Commy is doing", () => {
  assert.deepEqual(keyLook("mute", {}, status), { state: 1, title: "" });
  assert.deepEqual(keyLook("deafen", {}, status), { state: 0, title: "" });
  assert.deepEqual(keyLook("stream", {}, status), { state: 1, title: "" });
  assert.deepEqual(keyLook("push-to-talk", {}, status), { state: 0, title: "Off" });
  assert.deepEqual(keyLook("input-mode", {}, status), { state: 0, title: "Voice\nactivity" });
  assert.deepEqual(keyLook("voice-channel", { server: "s1", channel: "c1", label: "Lounge" }, status), { state: 1, title: "Lounge" });
  assert.deepEqual(keyLook("voice-channel", { server: "s1", channel: "c2", label: "Games" }, status), { state: 0, title: "Games" });
  assert.deepEqual(keyLook("text-channel", { label: "general" }, status), { state: 0, title: "#general" });
  assert.deepEqual(keyLook("volume", { target: "input" }, status), { state: 0, title: "Mic\n120%" });
  assert.deepEqual(keyLook("user-volume", { user: "u1", label: "Sam" }, status), { state: 0, title: "Sam\n50%" });
  assert.deepEqual(keyLook("user-volume", { user: "u2", label: "Alex" }, status), { state: 0, title: "Alex\n100%" });
  assert.deepEqual(keyLook("audio-device", { target: "output", device: "Desk speakers", label: "Desk speakers" }, status).state, 1);
  assert.deepEqual(keyLook("audio-device", { target: "input", device: "", label: "System default" }, status).state, 1);
  assert.deepEqual(keyLook("server-stats", { server: "s1", label: "Friends" }, status), { state: 0, title: "Friends\n12/40\nonline" });
  assert.deepEqual(keyLook("notifications", {}, status), { state: 1, title: "4" });
});

test("without Commy, keys fall back to their saved names", () => {
  assert.deepEqual(keyLook("mute", {}, null), { state: 0, title: "" });
  assert.deepEqual(keyLook("voice-channel", { label: "Lounge" }, null), { state: 0, title: "Lounge" });
});
