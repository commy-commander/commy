import assert from "node:assert/strict";
import test from "node:test";

import type { SoundboardList } from "../src/protocol.ts";
import { chosenSound, keyTitle, playTarget, stateMessage, wrapTitle } from "../src/sound-keys.ts";

const list: SoundboardList = {
  voice_server: "s1",
  defaults: [{ name: "default:airhorn", label: "Airhorn", emoji: null }],
  servers: [{ id: "s1", name: "Friends", can_play: true, can_add: false, sounds: [{ name: "boom", label: "boom", emoji: "💥" }] }],
};

test("titles wrap on words and stay within three lines", () => {
  assert.deepEqual(wrapTitle("Ba Dum Tss"), ["Ba Dum", "Tss"]);
  assert.deepEqual(wrapTitle("Airhorn"), ["Airhorn"]);
  assert.deepEqual(wrapTitle("supercalifragilistic"), ["supercali", "fragilist", "ic"]);
  const long = wrapTitle("one two three four five six seven eight");
  assert.equal(long.length, 3);
  assert.ok(long[2].endsWith("…"));
  assert.ok(long.every(line => line.length <= 9));
});

test("a key shows its emoji above the sound name", () => {
  assert.equal(keyTitle({ sound: "boom", label: "boom", emoji: "💥" }), "💥\nboom");
  assert.equal(keyTitle({ sound: "default:airhorn", label: "Airhorn" }), "Airhorn");
  assert.equal(keyTitle({}), "");
});

test("a key with no server plays in the voice server", () => {
  assert.deepEqual(playTarget({ sound: "default:airhorn", server: "" }), { sound: "default:airhorn" });
  assert.deepEqual(playTarget({ sound: "boom", server: "s1" }), { sound: "boom", server: "s1" });
  assert.equal(playTarget({ server: "s1" }), null);
});

test("server sounds are found only in their own server", () => {
  assert.deepEqual(chosenSound(list, { server: "s1", sound: "boom" }), { server: "s1", sound: "boom", label: "boom", emoji: "💥" });
  assert.equal(chosenSound(list, { server: "", sound: "boom" }), null);
  assert.equal(chosenSound(list, { server: "s1", sound: "gone" }), null);
  assert.equal(chosenSound(list, { sound: "default:airhorn" })?.label, "Airhorn");
});

test("every state that is not ready explains itself", () => {
  for (const state of ["offline", "waiting", "declined", "busy", "unsupported"] as const) {
    assert.ok(stateMessage(state).length > 0, state);
  }
  assert.equal(stateMessage("ready"), "");
});
