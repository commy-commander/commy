import assert from "node:assert/strict";
import test from "node:test";

import {
  fileStem,
  findSound,
  preferredUploadServer,
  serverOptions,
  settingsFor,
  soundEmoji,
  soundGroups,
  UPLOAD_PART_CHARS,
  uploadParts,
  uploadServers,
  VOICE_SERVER,
} from "../chat.commy.soundboard.sdPlugin/ui/inspector-options.js";

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

test("only servers that let you add sounds are offered for uploads, voice server first", () => {
  const upload = {
    voice_server: "s3",
    defaults: [],
    servers: [
      { id: "s1", name: "Friends", can_play: true, can_add: true, sounds: [] },
      { id: "s2", name: "Work", can_play: true, can_add: false, sounds: [] },
      { id: "s3", name: "Zoo", can_play: true, can_add: true, sounds: [] },
    ],
  };
  const options = uploadServers(upload);
  assert.deepEqual(options, [{ value: "s3", label: "Zoo" }, { value: "s1", label: "Friends" }]);
  assert.equal(preferredUploadServer(options, { server: "s1" }), "s1");
  assert.equal(preferredUploadServer(options, { server: "s2" }), "s3");
  assert.equal(preferredUploadServer([], { server: "s1" }), "");
});

test("uploads are split into parts that join back to the same data", () => {
  const data = "x".repeat(UPLOAD_PART_CHARS * 2 + 5);
  const parts = uploadParts(data);
  assert.equal(parts.length, 3);
  assert.equal(parts.join(""), data);
  assert.deepEqual(uploadParts(""), [""]);
});

test("file names become a starting sound name and empty emoji is left out", () => {
  assert.equal(fileStem("C:\\fakepath\\Big Boom.mp3"), "Big Boom");
  assert.equal(fileStem("/Users/me/clip.final.wav"), "clip.final");
  assert.equal(fileStem(".hidden"), ".hidden");
  assert.equal(soundEmoji("  "), null);
  assert.equal(soundEmoji(" 💥 "), "💥");
});
