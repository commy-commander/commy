import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  PROFILE_POLICIES, PROFILE_DETAIL_FIELDS, FRIEND_NOTE_MAX, canSeeProfile, publicProfile, cleanFriendNote, friendNoteState, friendRequestLine,
} = require("../scripts/profile-privacy.js");

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const read = (...path) => readFileSync(join(repo, ...path), "utf8");

test("the three policies match the backend's names and the settings control's options", () => {
  const rust = read("apps", "desktop", "src-tauri", "src", "safety.rs");
  for (const { value } of PROFILE_POLICIES) assert.match(rust, new RegExp(`"${value}" => Some\\(Self::`));
  const html = read("apps", "desktop", "ui", "index.html");
  const select = /<select id="profile-policy"[\s\S]*?<\/select>/.exec(html)[0];
  const options = [...select.matchAll(/<option value="([^"]+)">([^<]+)<\/option>/g)].map(m => ({ value: m[1], label: m[2] }));
  assert.deepEqual(options, PROFILE_POLICIES);
  assert.equal(Number(/pub const MAX_FRIEND_NOTE_CHARS: usize = (\d+);/.exec(rust)[1]), FRIEND_NOTE_MAX);
});

test("who sees the profile depends on the policy and how the viewer relates to us", () => {
  const stranger = {};
  const member = { sharesServer: true };
  const friend = { friend: true };
  assert.deepEqual([stranger, member, friend].map(v => canSeeProfile("everyone", v)), [true, true, true]);
  assert.deepEqual([stranger, member, friend].map(v => canSeeProfile("shared_servers", v)), [false, true, true]);
  assert.deepEqual([stranger, member, friend].map(v => canSeeProfile("friends", v)), [false, false, true]);
  assert.equal(canSeeProfile("friends", { friend: true, sharesServer: true }), true);
  assert.equal(canSeeProfile(undefined, null), true, "no policy means the open default");
});

test("a restricted profile keeps only what chat needs", () => {
  const person = {
    id: "u1", name: "nova", username: "nova", avatar: "emoji:x", presence: "idle", custom_status: "around",
    about: "bio", banner: "blob:b", pronouns: "they / them", activity: "Playing Chess", activity_cover: "co1", activity_since: 12,
  };
  const bare = publicProfile(person, false);
  assert.deepEqual(bare, { id: "u1", name: "nova", username: "nova", avatar: "emoji:x", presence: "idle", custom_status: "around" });
  for (const field of PROFILE_DETAIL_FIELDS) assert.equal(field in bare, false);
  assert.equal(publicProfile(person, true), person);
  assert.equal(publicProfile(null, false), null);
});

test("a friend note is one trimmed line of at most 200 characters", () => {
  assert.equal(cleanFriendNote("  we met\nat the  raid\t"), "we met at the raid");
  assert.equal(cleanFriendNote("a\u0007b"), "a b");
  assert.equal(cleanFriendNote(""), "");
  assert.equal(cleanFriendNote(null), "");
  const long = "x".repeat(FRIEND_NOTE_MAX + 30);
  assert.equal([...cleanFriendNote(long)].length, FRIEND_NOTE_MAX);
  const emoji = "\u{1f600}".repeat(FRIEND_NOTE_MAX + 5);
  assert.equal([...cleanFriendNote(emoji)].length, FRIEND_NOTE_MAX, "counts characters, not bytes");
});

test("the live counter follows the cleaned note", () => {
  assert.deepEqual(friendNoteState(""), { text: null, used: 0, counter: "0/200", ok: true });
  assert.deepEqual(friendNoteState(" hi there "), { text: "hi there", used: 8, counter: "8/200", ok: true });
  assert.equal(friendNoteState("y".repeat(500)).counter, "200/200");
  assert.equal(friendNoteState("y".repeat(500)).ok, true);
});

test("the incoming row shows the note when there is one", () => {
  assert.equal(friendRequestLine({ id: "u", name: "nova" }), "Wants to be friends");
  assert.equal(friendRequestLine({ id: "u", name: "nova", note: "" }), "Wants to be friends");
  assert.equal(friendRequestLine({ id: "u", name: "nova", note: " raid\nbuddy " }), "raid buddy");
  assert.equal(friendRequestLine(null), "Wants to be friends");
});
