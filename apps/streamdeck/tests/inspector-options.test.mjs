import assert from "node:assert/strict";
import test from "node:test";

import { findSound, serverOptions, settingsFor, soundGroups, VOICE_SERVER } from "../chat.commy.soundboard.sdPlugin/ui/inspector-options.js";

const list = {
  voice_server: null,
  defaults: [{ name: "default:airhorn", label: "Airhorn", emoji: null }],
  servers: [
    { id: "s1", name: "Friends", can_play: true, sounds: [{ name: "boom", label: "boom", emoji: "💥" }] },
    { id: "s2", name: "Work", can_play: false, sounds: [] },
  ],
};

test("the voice server comes first and servers without access are disabled", () => {
  const options = serverOptions(list, "");
  assert.equal(options[0].value, VOICE_SERVER);
  assert.deepEqual(options.map(o => o.disabled), [false, false, true]);
});

test("a server the user left stays visible so the key keeps its setting", () => {
  const options = serverOptions(list, "gone");
  assert.equal(options.at(-1)?.value, "gone");
  assert.equal(options.at(-1)?.disabled, true);
});

test("server sounds come before the built in ones", () => {
  assert.deepEqual(soundGroups(list, "s1").map(g => g.label), ["Friends", "Built in"]);
  assert.deepEqual(soundGroups(list, "").map(g => g.label), ["Built in"]);
  assert.deepEqual(soundGroups(list, "s2").map(g => g.label), ["Built in"]);
});

test("saved settings carry the label and emoji the key shows", () => {
  const groups = soundGroups(list, "s1");
  assert.deepEqual(settingsFor("s1", findSound(groups, "boom")), { server: "s1", sound: "boom", label: "boom", emoji: "💥" });
  assert.deepEqual(settingsFor("s1", findSound(groups, "missing")), { server: "s1" });
});
