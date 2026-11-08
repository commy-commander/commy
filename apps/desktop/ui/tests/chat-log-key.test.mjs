import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { chatLogKey } = require("../scripts/chat-log-key.js");

const messages = [{ id: "m1", author_id: "u1", content: "hi", reactions: [] }];

function server(overrides = {}) {
  return {
    id: "s1",
    name: "Crew",
    channels: [{ id: "c1", name: "general", kind: "text", voice_user_ids: [], voice_muted: [], speakers: [], hands: [] }],
    members: [{ id: "u1", name: "Ash", roles: [], voice_channel: null, voice_muted: false, voice_deafened: false, presence: "online" }],
    screen_shares: [],
    cameras: [],
    server_muted: [],
    server_deafened: [],
    ...overrides,
  };
}

test("muting, deafening or leaving voice leaves the chat log untouched", () => {
  const before = chatLogKey("c1", null, messages, server());
  const muted = server({
    channels: [{ id: "c1", name: "general", kind: "text", voice_user_ids: ["u1"], voice_muted: [true], speakers: ["u1"], hands: [] }],
    members: [{ id: "u1", name: "Ash", roles: [], voice_channel: "c9", voice_muted: true, voice_deafened: true, presence: "idle" }],
    screen_shares: [{ user: "u1", name: "Ash", channel: "c9" }],
    cameras: ["u1"],
    server_muted: ["u1"],
  });
  assert.equal(chatLogKey("c1", null, messages, muted), before);
});

test("a new, edited or reacted message redraws the chat log", () => {
  const before = chatLogKey("c1", null, messages, server());
  assert.notEqual(chatLogKey("c1", null, [...messages, { id: "m2", author_id: "u1", content: "yo" }], server()), before);
  assert.notEqual(chatLogKey("c1", null, [{ ...messages[0], content: "hello" }], server()), before);
  assert.notEqual(chatLogKey("c1", null, [{ ...messages[0], reactions: [{ emoji: "x", users: ["u1"] }] }], server()), before);
});

test("a renamed member or changed role redraws the chat log", () => {
  const before = chatLogKey("c1", null, messages, server());
  assert.notEqual(chatLogKey("c1", null, messages, server({ members: [{ id: "u1", name: "Ashley", roles: [] }] })), before);
  assert.notEqual(chatLogKey("c1", null, messages, server({ roles: [{ id: "r1", color: "#f00" }] })), before);
});

test("another channel or a moved read line redraws the chat log", () => {
  const before = chatLogKey("c1", null, messages, server());
  assert.notEqual(chatLogKey("c2", null, messages, server()), before);
  assert.notEqual(chatLogKey("c1", "m1", messages, server()), before);
});

test("a missing server still gives a stable key", () => {
  assert.equal(chatLogKey("c1", null, messages, null), chatLogKey("c1", null, messages, null));
});
