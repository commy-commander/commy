import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { DEFAULT_NOTIFY_LEVEL, RECENT_SECS, MUTE_CHOICES, shouldNotify, noticeText, messageNotice, mentionsName, levelForServer, levelForChannel, isMuted, channelPref, withChannelPref, muteUntil, isRecent, caughtUpNotice, mentionReaches, withServerFilter, filterOn, phoneInFront } =
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

test("a message that is only a voice message is announced as one", () => {
  const msg = { author_name: "nova", channel_name: "general", server_name: "Rust Hangout", content: "", has_attachments: true, voice_message: true };
  assert.equal(messageNotice(msg, plain).body, "Voice message");
  assert.equal(messageNotice({ ...msg, content: "hear this" }, plain).body, "hear this");
  assert.equal(messageNotice({ ...msg, voice_message: false }, plain).body, "Sent an attachment");
  assert.equal(caughtUpNotice({ ...msg, count: 3 }, plain).body, "Voice message · 2 more messages");
  assert.equal(noticeText("", false, plain, true), "Voice message");
  assert.equal(noticeText("", false, plain, false), "");
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

test("a channel follows its server until it has a level of its own", () => {
  const prefs = { level: "all", servers: { s1: "mentions" }, channels: { s1: { c1: { level: "none" }, c2: { level: "default" } } } };
  assert.equal(levelForChannel(prefs, "s1", "c1", 0), "none");
  assert.equal(levelForChannel(prefs, "s1", "c2", 0), "mentions");
  assert.equal(levelForChannel(prefs, "s1", "c3", 0), "mentions");
  assert.equal(levelForChannel(prefs, "s2", "c1", 0), "all");
  assert.equal(levelForChannel({ servers: {} }, "s2", "c1", 0), "all");
  assert.equal(channelPref(prefs, "s1", "c1").level, "none");
  assert.equal(channelPref(prefs, "s9", "c1"), null);
});

test("a muted channel or server silences the channel, and a timed mute ends on its own", () => {
  const now = 1_000_000;
  const prefs = {
    level: "all",
    servers: {},
    muted: { gone: true },
    channels: { s1: { forever: { mute: true }, soon: { mute: now + 60 }, over: { mute: now - 1 }, exact: { mute: now }, odd: { mute: "yes" }, level: { level: "all" } } },
  };
  assert.equal(isMuted(prefs, "s1", "forever", now), true);
  assert.equal(isMuted(prefs, "s1", "soon", now), true);
  assert.equal(isMuted(prefs, "s1", "over", now), false);
  assert.equal(isMuted(prefs, "s1", "exact", now), false);
  assert.equal(isMuted(prefs, "s1", "odd", now), false);
  assert.equal(isMuted(prefs, "s1", "level", now), false);
  assert.equal(isMuted(prefs, "s1", "unknown", now), false);
  assert.equal(isMuted(prefs, "gone", "anything", now), true);
  assert.equal(isMuted({ servers: {} }, "s1", "c1", now), false);
  assert.equal(levelForChannel(prefs, "s1", "forever", now), "none");
  assert.equal(levelForChannel(prefs, "s1", "soon", now), "none");
  assert.equal(levelForChannel(prefs, "s1", "over", now), "all");
  assert.equal(levelForChannel(prefs, "gone", "c1", now), "none");
  assert.equal(shouldNotify({ level: levelForChannel(prefs, "s1", "soon", now), presence: "online", focusedOnIt: false, isDm: false, mentioned: true }), false);
});

test("channel choices are stored per server and cleaned up when they return to the default", () => {
  const now = 1_000_000;
  let prefs = { level: "all", servers: {} };
  prefs = withChannelPref(prefs, "s1", "c1", { level: "mentions" });
  assert.deepEqual(prefs.channels, { s1: { c1: { level: "mentions" } } });
  prefs = withChannelPref(prefs, "s1", "c1", { mute: muteUntil(MUTE_CHOICES[0], now) });
  assert.deepEqual(prefs.channels, { s1: { c1: { level: "mentions", mute: now + 15 * 60 } } });
  prefs = withChannelPref(prefs, "s1", "c1", { mute: muteUntil(MUTE_CHOICES[4], now) });
  assert.equal(prefs.channels.s1.c1.mute, true);
  prefs = withChannelPref(prefs, "s1", "c1", { level: "default" });
  assert.deepEqual(prefs.channels, { s1: { c1: { mute: true } } });
  prefs = withChannelPref(prefs, "s1", "c1", { mute: false });
  assert.deepEqual(prefs.channels, {});
  assert.equal(prefs.level, "all");
  assert.deepEqual(MUTE_CHOICES.map(c => c.seconds), [900, 3600, 28800, 86400, null]);
});

test("a server can ignore @everyone and @here or role mentions while @you always counts", () => {
  let prefs = { servers: {} };
  const everyone = { mentions: ["everyone"], mentioned: true };
  const role = { mentions: ["role"], mentioned: true };
  const both = { mentions: ["role", "everyone"], mentioned: true };
  const named = { mentions: ["name", "everyone"], mentioned: true };
  assert.equal(mentionReaches(prefs, "s1", everyone), true);
  prefs = withServerFilter(prefs, "s1", "suppressEveryone", true);
  assert.equal(filterOn(prefs, "suppressEveryone", "s1"), true);
  assert.equal(mentionReaches(prefs, "s1", everyone), false);
  assert.equal(mentionReaches(prefs, "s2", everyone), true, "other servers keep their own setting");
  assert.equal(mentionReaches(prefs, "s1", both), true, "a role you hold still reaches you");
  prefs = withServerFilter(prefs, "s1", "suppressRoles", true);
  assert.equal(mentionReaches(prefs, "s1", role), false);
  assert.equal(mentionReaches(prefs, "s1", both), false);
  assert.equal(mentionReaches(prefs, "s1", named), true);
  assert.equal(mentionReaches(prefs, "s1", { mentions: [], mentioned: false }), false);
  assert.equal(mentionReaches(prefs, "s1", { mentioned: true }), true, "older builds only say whether you were mentioned");
  prefs = withServerFilter(prefs, "s1", "suppressEveryone", false);
  assert.deepEqual(prefs.suppressEveryone, {});
  assert.equal(withServerFilter(prefs, "s1", "level", true), prefs, "only the mention filters are set here");
});

test("the phone app stays quiet while it is on screen and notifies once it is not", () => {
  assert.equal(phoneInFront(true, "visible"), true);
  assert.equal(phoneInFront(true, "hidden"), false);
  assert.equal(phoneInFront(false, "visible"), false);
  assert.equal(shouldNotify({ level: "all", presence: "online", focusedOnIt: false, phoneInFront: true, isDm: true, mentioned: true }), false);
  assert.equal(shouldNotify({ level: "all", presence: "online", focusedOnIt: false, phoneInFront: false, isDm: true, mentioned: true }), true);
});
