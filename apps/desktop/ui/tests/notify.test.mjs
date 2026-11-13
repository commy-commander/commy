import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { DEFAULT_NOTIFY_LEVEL, RECENT_SECS, MUTE_CHOICES, shouldNotify, noticeText, messageNotice, mentionsName, levelForServer, levelForChannel, isMuted, channelPref, withChannelPref, muteUntil, isRecent, caughtUpNotice, mentionReaches, withServerFilter, filterOn, phoneInFront, alertWay, playsAppSound, inAppLine, sentBeforeFront, noticeTarget, quickReplyCard, anywayWay, anywayNotice, pausedMentions, pausedLine } =
  require("../scripts/notify.js");

const plain = s => s;

test("a plain message from a friend notifies out of the box", () => {
  assert.equal(DEFAULT_NOTIFY_LEVEL, "all");
  const prefs = { servers: {} };
  const level = levelForServer(prefs, "s1");
  assert.equal(level, "all");
  assert.equal(shouldNotify({ level, presence: "online", focusedOnIt: false, isDm: false, mentioned: false }), true);
});

test("idle, appearing offline or tucked away in the tray still notifies out of the box", () => {
  for (const presence of ["idle", "invisible"]) {
    assert.equal(shouldNotify({ level: DEFAULT_NOTIFY_LEVEL, presence, focusedOnIt: false, isDm: false, mentioned: false }), true, presence);
    assert.equal(shouldNotify({ level: DEFAULT_NOTIFY_LEVEL, presence, focusedOnIt: false, isDm: true, mentioned: true }), true, presence);
  }
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
  assert.equal(messageNotice({ ...msg, channel_name: "chat", server_name: "" }, strip).title, "ferris", "a group with no name shows just the sender");
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

test("the phone app alerts inside the app while it is on screen and with a system notice once it is not", () => {
  assert.equal(phoneInFront(true, "visible"), true);
  assert.equal(phoneInFront(true, "hidden"), false);
  assert.equal(phoneInFront(false, "visible"), false);
  const dm = { level: "all", presence: "online", focusedOnIt: false, isDm: true, mentioned: true };
  assert.equal(alertWay({ ...dm, phoneInFront: true }), "in_app");
  assert.equal(alertWay({ ...dm, phoneInFront: false }), "system");
  assert.equal(alertWay({ ...dm, phoneInFront: true, focusedOnIt: true }), "none");
  assert.equal(alertWay({ ...dm, phoneInFront: true, presence: "do_not_disturb" }), "none");
  assert.equal(alertWay({ level: "mentions", presence: "online", focusedOnIt: false, phoneInFront: true, isDm: false, mentioned: false }), "none");
  assert.equal(alertWay({ level: "mentions", presence: "online", focusedOnIt: false, phoneInFront: true, isDm: false, mentioned: true }), "in_app");
});

test("opening the phone app stays quiet about messages its push already announced", () => {
  const dm = { level: "all", presence: "online", focusedOnIt: false, isDm: true, mentioned: true };
  assert.equal(alertWay({ ...dm, phoneInFront: true, at: 99, frontSince: 100 }), "none", "sent while the app was away");
  assert.equal(alertWay({ ...dm, phoneInFront: true, at: 100, frontSince: 100 }), "in_app", "sent once the app was open");
  assert.equal(alertWay({ ...dm, phoneInFront: true, caughtUp: true }), "none", "caught up on opening");
  assert.equal(alertWay({ ...dm, phoneInFront: false, at: 99, frontSince: 100 }), "system", "the desktop and a phone in the background are unchanged");
  assert.equal(alertWay({ ...dm, phoneInFront: false, caughtUp: true }), "system");
  assert.equal(alertWay({ ...dm, phoneInFront: true, at: NaN, frontSince: 100 }), "in_app", "older backends send no time");
  assert.equal(sentBeforeFront(undefined, 100), false);
  assert.equal(sentBeforeFront(5, undefined), false);
});

test("a phone in the background lets its notification play the sound, so it never plays twice", () => {
  assert.equal(playsAppSound("in_app", true), true);
  assert.equal(playsAppSound("system", true), false);
  assert.equal(playsAppSound("system", false), true, "the desktop keeps its own alert sound");
  assert.equal(playsAppSound("in_app", false), true);
  assert.equal(playsAppSound("none", false), false);
  assert.equal(playsAppSound("none", true), false);
});

test("an in-app alert reads as one line", () => {
  assert.equal(inAppLine({ title: "Ana in #general (Club)", body: "hi" }), "Ana in #general (Club): hi");
  assert.equal(inAppLine({ title: "Ana", body: "" }), "Ana");
});

test("a tapped notification names the DM or the channel it opens", () => {
  const id = "7da1b7e3-9818-4075-a087-1c4587540659";
  const room = "0b1c2d3e-4f50-4617-8899-aabbccddeeff";
  assert.deepEqual(noticeTarget(`dm:${id}`), { kind: "dm", user: id });
  assert.deepEqual(noticeTarget(`channel:${id}:${room}`), { kind: "channel", server: id, channel: room });
  for (const bad of [null, "", "dm:", "dm:a b", `dm:${id}:x`, `channel:${id}`, `channel:${id}:${room}:x`, "call:abc", "open:calc.exe"]) {
    assert.equal(noticeTarget(bad), null, String(bad));
  }
});

test("phones get a tappable banner instead of the quick reply card", () => {
  assert.equal(quickReplyCard({ focused: true, viewing: false, phone: false }), true);
  assert.equal(quickReplyCard({ focused: true, viewing: false, phone: true }), false);
  assert.equal(quickReplyCard({ focused: true, viewing: true, phone: false }), false);
  assert.equal(quickReplyCard({ focused: false, viewing: false, phone: false }), false);
});

test("notify anyway gets through Do Not Disturb and only then", () => {
  const base = { level: "mentions", focusedOnIt: false, phoneInFront: false };
  assert.equal(anywayWay({ ...base, presence: "do_not_disturb" }), "system");
  assert.equal(anywayWay({ ...base, presence: "do_not_disturb", phoneInFront: true }), "in_app");
  assert.equal(anywayWay({ ...base, presence: "online" }), "none", "they already heard the message itself");
  assert.equal(anywayWay({ ...base, presence: "do_not_disturb", level: "none" }), "none", "a muted conversation stays muted");
  assert.equal(anywayWay({ ...base, presence: "do_not_disturb", focusedOnIt: true }), "none");
});

test("a notify anyway says what it is", () => {
  const plain = text => text;
  assert.deepEqual(anywayNotice({ author_name: "mia", content: "are you up?" }, plain), { title: "mia", body: "Notify anyway: are you up?" });
  assert.deepEqual(anywayNotice({ author_name: "mia", channel_name: "general", server_name: "Hangout", content: "@sam look" }, plain), { title: "mia (#general, Hangout)", body: "Notify anyway: @sam look" });
  assert.equal(anywayNotice({ author_name: "mia", content: "" }, plain).body, "Wants to reach you");
});

test("the prompt offers only paused people your message names", () => {
  const members = [
    { id: "me", username: "me", name: "Me", presence: "do_not_disturb" },
    { id: "a", username: "sam", name: "Sam", presence: "do_not_disturb" },
    { id: "b", username: "kit", name: "Kit", nickname: "kitty", presence: "do_not_disturb" },
    { id: "c", username: "lee", name: "Lee", presence: "online" },
  ];
  assert.deepEqual(pausedMentions("@sam and @kitty and @lee and @me", members, "me").map(m => m.id), ["a", "b"]);
  assert.deepEqual(pausedMentions("no names here", members, "me"), []);
  assert.deepEqual(pausedMentions("@sam", undefined, "me"), []);
});

test("the prompt names who has notifications paused", () => {
  assert.equal(pausedLine(["Sam"]), "Sam has notifications paused");
  assert.equal(pausedLine(["Sam", "Kit"]), "Sam and Kit have notifications paused");
  assert.equal(pausedLine(["Sam", "Kit", "Lee"]), "Sam, Kit and Lee have notifications paused");
  assert.equal(pausedLine([]), "");
});
