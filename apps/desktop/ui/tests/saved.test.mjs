import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  MAX_SAVED_MESSAGES, REMINDER_PRESETS, reminderDue, parseReminderTime, dueReminders, sortReminders, sortSaved,
  messageEntry, savedOrigin, savedExcerpt, savedTarget, reminderLabel, reminderNotice, savedButtonTitle, savedSummary, toDateTimeLocal,
} = require("../scripts/saved.js");

const noon = new Date(2026, 9, 20, 12, 0, 0, 0).getTime();

test("presets compute a due time from now, and tomorrow means tomorrow morning", () => {
  assert.equal(reminderDue("20m", noon), noon + 20 * 60_000);
  assert.equal(reminderDue("1h", noon), noon + 3_600_000);
  assert.equal(reminderDue("3h", noon), noon + 3 * 3_600_000);
  assert.equal(reminderDue("tomorrow", noon), new Date(2026, 9, 21, 9, 0, 0, 0).getTime());
  assert.equal(reminderDue("custom", noon), null);
  assert.equal(reminderDue("nope", noon), null);
  assert.deepEqual(REMINDER_PRESETS.map(p => p.id), ["20m", "1h", "3h", "tomorrow", "custom"]);
});

test("a picked time is read from the picker's format and must be in the future", () => {
  assert.equal(parseReminderTime("2026-10-20T18:30", noon), new Date(2026, 9, 20, 18, 30).getTime());
  assert.equal(parseReminderTime("2026-10-20T11:59", noon), null);
  assert.equal(parseReminderTime("2026-10-20T12:00", noon), null);
  assert.equal(parseReminderTime("2026-02-30T12:00", noon), null);
  assert.equal(parseReminderTime("tomorrow", noon), null);
  assert.equal(parseReminderTime("", noon), null);
  assert.equal(parseReminderTime(toDateTimeLocal(noon + 90_000), noon), noon + 60_000);
});

test("due reminders are the ones at or before now, and upcoming ones sort soonest first", () => {
  const reminders = [
    { id: "b", due: noon + 1000 },
    { id: "a", due: noon - 1000 },
    { id: "c", due: noon },
  ];
  assert.deepEqual(dueReminders(reminders, noon).map(r => r.id), ["a", "c"]);
  assert.deepEqual(sortReminders(reminders).map(r => r.id), ["a", "c", "b"]);
  assert.deepEqual(dueReminders(null, noon), []);
});

test("saved messages list newest saved first", () => {
  const saved = [
    { message: "m1", saved_at: 1 },
    { message: "m3", saved_at: 3 },
    { message: "m2", saved_at: 2 },
  ];
  assert.deepEqual(sortSaved(saved).map(s => s.message), ["m3", "m2", "m1"]);
  assert.equal(MAX_SAVED_MESSAGES, 500);
});

test("an entry remembers where a message came from, for channels and for direct messages", () => {
  const message = { id: "x1", author_id: "m1", author_name: "ferris", content: "hello", created_at: "2026-10-20T10:00:00Z", attachments: [{ filename: "notes.txt" }] };
  const channelEntry = messageEntry(message, { server: { id: "s", name: "Rust Hangout" }, channel: { id: "c", name: "general" } });
  assert.equal(channelEntry.server, "s");
  assert.equal(channelEntry.channel_name, "general");
  assert.equal(channelEntry.dm, null);
  assert.equal(channelEntry.attachment, "notes.txt");
  assert.equal(savedOrigin(channelEntry), "Rust Hangout · #general");
  assert.equal(savedTarget(channelEntry), "channel:s:c");

  const dmEntry = messageEntry(message, { server: { id: "s", name: "ignored" }, dm: { user: "m1", name: "ferris" } });
  assert.equal(dmEntry.server, null);
  assert.equal(dmEntry.dm, "m1");
  assert.equal(savedOrigin(dmEntry), "Direct message with ferris");
  assert.equal(savedTarget(dmEntry), "dm:m1");
  assert.equal(savedTarget({ message: "x" }), null);
});

test("the excerpt strips markdown and falls back to the attachment", () => {
  assert.equal(savedExcerpt({ content: "## Patch notes\n**bold** and `code`" }), "Patch notes bold and code");
  assert.equal(savedExcerpt({ content: "", attachment: "clip.mp4" }), "Attachment: clip.mp4");
  assert.equal(savedExcerpt({ content: "" }), "No text");
  assert.equal(savedExcerpt({ content: "<b>x</b>" }, () => "plain"), "plain");
});

test("reminder chips read naturally", () => {
  assert.equal(reminderLabel(noon - 1, noon), "Due now");
  assert.equal(reminderLabel(noon + 20 * 60_000, noon), "In 20 min");
  assert.match(reminderLabel(noon + 3 * 3_600_000, noon), /^Today /);
  assert.match(reminderLabel(new Date(2026, 9, 21, 9, 0).getTime(), noon), /^Tomorrow /);
  assert.match(reminderLabel(new Date(2026, 10, 2, 9, 0).getTime(), noon), /^Nov 2 /);
});

test("a due reminder becomes a notification naming the author and the channel", () => {
  const entry = { author_name: "ferris", channel_name: "general", content: "raid night friday?" };
  assert.deepEqual(reminderNotice({ entry }), { title: "Reminder: ferris in #general", body: "raid night friday?" });
  assert.equal(reminderNotice({ entry: { ...entry, channel_name: null, dm: "m1" } }).title, "Reminder: ferris");
  assert.equal(reminderNotice(null).title, "Reminder");
});

test("the saved button counts only upcoming reminders", () => {
  assert.equal(savedButtonTitle([], noon), "Saved Messages");
  assert.equal(savedButtonTitle([{ due: noon - 1 }], noon), "Saved Messages");
  assert.equal(savedButtonTitle([{ due: noon + 1 }], noon), "Saved Messages · 1 reminder");
  assert.equal(savedButtonTitle([{ due: noon + 1 }, { due: noon + 2 }], noon), "Saved Messages · 2 reminders");
});

test("the sidebar entry sums saved messages and upcoming reminders", () => {
  assert.equal(savedSummary([], [], noon), "Nothing saved yet");
  assert.equal(savedSummary([{}], [], noon), "1 message");
  assert.equal(savedSummary([{}, {}], [{ due: noon + 1 }], noon), "2 messages · 1 reminder");
  assert.equal(savedSummary([], [{ due: noon - 1 }, { due: noon + 1 }, { due: noon + 2 }], noon), "2 reminders");
});
