import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { DEFAULT_NOTIFY_LEVEL, RECENT_SECS, shouldNotify, messageNotice, mentionsName, levelForServer, isRecent, caughtUpNotice } =
  require("../scripts/notify.js");

const plain = s => s;

test("a plain message from a friend notifies out of the box", () => {
  assert.equal(DEFAULT_NOTIFY_LEVEL, "all");
  const prefs = { servers: {} };
  const level = levelForServer(prefs, "s1");
  assert.equal(level, "all");
  assert.equal(shouldNotify({ level, presence: "online", focusedOnIt: false, isDm: false, mentioned: false }), true);
});

test("looking at the conversation or Do Not Disturb keeps it quiet", () => {
  assert.equal(shouldNotify({ level: "all", presence: "online", focusedOnIt: true, isDm: false, mentioned: true }), false);
  assert.equal(shouldNotify({ level: "all", presence: "do_not_disturb", focusedOnIt: false, isDm: true, mentioned: true }), false);
  assert.equal(shouldNotify({ level: "none", presence: "online", focusedOnIt: false, isDm: false, mentioned: true }), false);
});

test("mentions level only pings on a mention, DMs always", () => {
  assert.equal(shouldNotify({ level: "mentions", presence: "online", focusedOnIt: false, isDm: false, mentioned: false }), false);
  assert.equal(shouldNotify({ level: "mentions", presence: "idle", focusedOnIt: false, isDm: false, mentioned: true }), true);
  assert.equal(shouldNotify({ level: "mentions", presence: "idle", focusedOnIt: false, isDm: true, mentioned: false }), true);
  assert.equal(shouldNotify({ level: "online", presence: "idle", focusedOnIt: false, isDm: false, mentioned: false }), false);
  assert.equal(shouldNotify({ level: "online", presence: "online", focusedOnIt: false, isDm: false, mentioned: false }), true);
});

test("announcements reach members who only want mentions, but never Do Not Disturb or a muted server", () => {
  const base = { presence: "online", focusedOnIt: false, isDm: false, mentioned: false, announcement: true };
  assert.equal(shouldNotify({ ...base, level: "mentions" }), true);
  assert.equal(shouldNotify({ ...base, level: "mentions", announcement: false }), false);
  assert.equal(shouldNotify({ ...base, level: "online" }), true);
  assert.equal(shouldNotify({ ...base, level: "online", presence: "idle" }), false);
  assert.equal(shouldNotify({ ...base, level: "none" }), false);
  assert.equal(shouldNotify({ ...base, level: "mentions", presence: "do_not_disturb" }), false);
  assert.equal(shouldNotify({ ...base, level: "mentions", focusedOnIt: true }), false);
});

test("an announcement is named as one in its notification", () => {
  const msg = { author_name: "ferris", channel_name: "news", server_name: "Rust Hangout", content: "**Patch day**", has_attachments: false };
  const strip = s => s.replace(/\*\*/g, "");
  assert.deepEqual(messageNotice({ ...msg, announcement: true }, strip), { title: "Announcement in #news (Rust Hangout)", body: "ferris: Patch day" });
  assert.deepEqual(messageNotice(msg, strip), { title: "ferris (#news, Rust Hangout)", body: "Patch day" });
  assert.equal(messageNotice({ ...msg, content: "", has_attachments: true }, strip).body, "Sent an attachment");
});

test("muted and per-server choices win over the global level", () => {
  const prefs = { level: "all", servers: { quiet: "mentions", normal: "default" }, muted: { gone: true } };
  assert.equal(levelForServer(prefs, "quiet"), "mentions");
  assert.equal(levelForServer(prefs, "normal"), "all");
  assert.equal(levelForServer(prefs, "gone"), "none");
  assert.equal(levelForServer(prefs, "other"), "all");
});

test("mentions match whole names, case-insensitively", () => {
  assert.equal(mentionsName("hey @sam", ["sam", null]), true);
  assert.equal(mentionsName("hey @Sam!", ["sam"]), true);
  assert.equal(mentionsName("hey @samantha", ["sam"]), false);
  assert.equal(mentionsName("@everyone up", []), false, "everyone depends on the sender, so it is decided elsewhere");
  assert.equal(mentionsName("", ["sam"]), false);
  assert.equal(mentionsName("a.b @a.b", ["a.b"]), true);
});

test("caught-up messages count as live only while fresh", () => {
  const now = 1_000_000;
  assert.equal(isRecent(now - 30, now), true);
  assert.equal(isRecent(now - RECENT_SECS, now), true);
  assert.equal(isRecent(now - RECENT_SECS - 1, now), false);
  assert.equal(isRecent(NaN, now), false);
  assert.equal(isRecent(undefined, now), false);
});

test("one notice per channel names the newest message and the rest", () => {
  const base = { author_name: "nova", channel_name: "general", server_name: "Pixel Pioneers", has_attachments: false };
  assert.deepEqual(caughtUpNotice({ ...base, count: 1, content: "hi" }, plain), {
    title: "nova (#general, Pixel Pioneers)",
    body: "hi",
  });
  assert.equal(caughtUpNotice({ ...base, count: 2, content: "hi" }, plain).body, "hi · 1 more message");
  assert.equal(caughtUpNotice({ ...base, count: 4, content: "hi" }, plain).body, "hi · 3 more messages");
  assert.equal(caughtUpNotice({ ...base, count: 1, content: "", has_attachments: true }, plain).body, "Sent an attachment");
  assert.equal(caughtUpNotice({ ...base, count: 3, content: "", author_name: "" }, plain).title, "Someone (#general, Pixel Pioneers)");
});
