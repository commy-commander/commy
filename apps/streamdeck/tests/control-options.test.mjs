import assert from "node:assert/strict";
import test from "node:test";

import {
  channelChoices,
  cleanAmount,
  deviceChoices,
  kindOf,
  labelFor,
  personChoices,
  serverChoices,
  withSaved,
} from "../chat.commy.soundboard.sdPlugin/ui/control-options.js";

const directory = {
  servers: [{ id: "s1", name: "Friends", text: [{ id: "t1", name: "general" }], voice: [{ id: "v1", name: "Lounge" }] }],
  people: [{ id: "u1", name: "Sam" }],
  inputs: ["Desk mic"],
  outputs: ["Desk speakers"],
};

test("the action decides which channel kind is offered", () => {
  assert.equal(kindOf("chat.commy.soundboard.voice-channel"), "voice-channel");
  assert.deepEqual(channelChoices(directory, "s1", "voice-channel"), [{ value: "v1", label: "Lounge" }]);
  assert.deepEqual(channelChoices(directory, "s1", "text-channel"), [{ value: "t1", label: "#general" }]);
  assert.deepEqual(channelChoices(directory, "gone", "text-channel"), []);
  assert.deepEqual(serverChoices(directory), [{ value: "s1", label: "Friends" }]);
  assert.deepEqual(personChoices(directory), [{ value: "u1", label: "Sam" }]);
});

test("devices start with the system default", () => {
  assert.deepEqual(deviceChoices(directory, "input").map(o => o.value), ["", "Desk mic"]);
  assert.deepEqual(deviceChoices(directory, "output").map(o => o.label), ["System default", "Desk speakers"]);
});

test("a saved choice that is gone stays visible but disabled", () => {
  const options = withSaved(serverChoices(directory), "s9", "Old place");
  assert.deepEqual(options.at(-1), { value: "s9", label: "Old place (not found)", disabled: true });
  assert.equal(labelFor(options, "s9"), undefined);
  assert.equal(withSaved(serverChoices(directory), "s1", "Friends").length, 1);
});

test("amounts are whole percents from 1 to 200", () => {
  assert.equal(cleanAmount("15"), 15);
  assert.equal(cleanAmount("0"), 1);
  assert.equal(cleanAmount("999"), 200);
  assert.equal(cleanAmount("lots"), 10);
});
