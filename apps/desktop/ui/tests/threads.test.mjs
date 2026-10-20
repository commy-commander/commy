import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  MAX_THREAD_NAME_CHARS, threadRootOf, threadReplyCounts, defaultThreadName, threadName, cleanThreadName,
  buildThreadList, canManageThread, threadComposerState, threadStatusLine, threadsButtonState,
} = require("../scripts/threads.js");

const at = minutesAgo => new Date(Date.UTC(2026, 9, 20, 12, 0) - minutesAgo * 60e3).toISOString();
const msg = (id, content, { reply_to = null, author_id = "m1", created_at = at(0), attachments = [] } = {}) =>
  ({ id, content, reply_to, author_id, created_at, attachments });

const messages = [
  msg("a", "where should we meet on friday for the raid night", { created_at: at(60) }),
  msg("b", "the clip from last night", { created_at: at(50), author_id: "m2" }),
  msg("a1", "pub?", { reply_to: "a", created_at: at(40) }),
  msg("a2", "sure", { reply_to: "a1", created_at: at(30), author_id: "m2" }),
  msg("c", "", { created_at: at(25), attachments: [{ filename: "map.png" }] }),
  msg("b1", "nice flick", { reply_to: "b", created_at: at(10) }),
  msg("lone", "hello", { created_at: at(5) }),
  msg("orphan", "replying to something gone", { reply_to: "gone", created_at: at(1) }),
];

test("a reply chain resolves to its root and counts land on the root", () => {
  const byId = new Map(messages.map(m => [m.id, m]));
  assert.equal(threadRootOf(byId.get("a2"), byId), "a");
  assert.equal(threadRootOf(byId.get("a"), byId), "a");
  assert.equal(threadRootOf(byId.get("orphan"), byId), "orphan");
  const counts = threadReplyCounts(messages);
  assert.equal(counts.get("a"), 2);
  assert.equal(counts.get("b"), 1);
  assert.equal(counts.has("lone"), false);
});

test("the default name takes the root's first words and marks a cut", () => {
  assert.equal(defaultThreadName("where should we meet on friday for the raid night"), "where should we meet on friday…");
  assert.equal(defaultThreadName("short one"), "short one");
  assert.equal(defaultThreadName("   spaced    out   "), "spaced out");
  assert.equal(defaultThreadName(""), "Thread");
  assert.equal(defaultThreadName("", "Attachment"), "Attachment");
  const long = defaultThreadName("supercalifragilisticexpialidocious".repeat(3));
  assert.ok(long.length <= 48 && long.endsWith("…"));
  assert.equal(threadName({ content: "**bold** plan", attachments: [] }, null, s => s.replace(/\*\*/g, "")), "bold plan");
  assert.equal(threadName({ content: "ignored" }, { name: "Raid night" }), "Raid night");
  assert.equal(threadName({ content: "", attachments: [{}] }, { name: null }), "Attachment");
  assert.equal(threadName(null, null), "Thread");
});

test("names are trimmed, collapsed and capped like the core does", () => {
  assert.deepEqual(cleanThreadName("  Raid   night "), { name: "Raid night" });
  assert.deepEqual(cleanThreadName("   "), { name: null });
  assert.equal(MAX_THREAD_NAME_CHARS, 100);
  assert.ok(cleanThreadName("x".repeat(101)).error);
  assert.deepEqual(cleanThreadName("x".repeat(100)), { name: "x".repeat(100) });
});

test("the list holds roots with replies or metadata, newest activity first, archived apart", () => {
  const threads = [
    { root: "c", name: "Map feedback", archived: false, locked: true, by: "m2" },
    { root: "a", name: null, archived: true, locked: false, by: "m1" },
    { root: "missing", name: "Not loaded", archived: false, locked: false, by: "m1" },
  ];
  const { active, archived } = buildThreadList(messages, threads);
  assert.deepEqual(active.map(t => t.id), ["b", "c"]);
  assert.deepEqual(archived.map(t => t.id), ["a"]);
  const b = active.find(t => t.id === "b");
  assert.deepEqual([b.name, b.replies, b.named, b.locked], ["the clip from last night", 1, false, false]);
  assert.equal(b.lastAt, Date.parse(at(10)));
  const c = active.find(t => t.id === "c");
  assert.deepEqual([c.name, c.replies, c.named, c.locked, c.lastAt], ["Map feedback", 0, true, true, Date.parse(at(25))]);
  assert.deepEqual([archived[0].replies, archived[0].name], [2, "where should we meet on friday…"]);
  assert.deepEqual(buildThreadList([], threads), { active: [], archived: [] });
  assert.deepEqual(buildThreadList(messages, null).active.map(t => t.id), ["b", "a"]);
});

test("the starter and moderators manage a thread; a locked thread closes the composer for everyone else", () => {
  const root = { author_id: "m1" };
  assert.equal(canManageThread(root, { userId: "m1", canManageMessages: false }), true);
  assert.equal(canManageThread(root, { userId: "m2", canManageMessages: false }), false);
  assert.equal(canManageThread(root, { userId: "m2", canManageMessages: true }), true);
  assert.equal(canManageThread(null, { userId: "m1", canManageMessages: true }), false);
  assert.deepEqual(threadComposerState({ meta: null }), { disabled: false, placeholder: "Reply in thread" });
  assert.deepEqual(threadComposerState({ meta: { locked: true }, canManageMessages: false }), { disabled: true, placeholder: "This thread is locked" });
  assert.deepEqual(threadComposerState({ meta: { locked: true }, canManageMessages: true }), { disabled: false, placeholder: "Reply in thread (locked for members)" });
  assert.deepEqual(threadComposerState({ meta: { locked: false, archived: true } }), { disabled: false, placeholder: "Reply in thread" });
});

test("the status line and the header button read their state in a few words", () => {
  assert.equal(threadStatusLine({ meta: null, replies: 0 }), "No replies yet");
  assert.equal(threadStatusLine({ meta: { locked: true }, replies: 1 }), "Locked · 1 reply");
  assert.equal(threadStatusLine({ meta: { archived: true, locked: true }, replies: 4 }), "Locked · Archived · 4 replies");
  assert.deepEqual(threadsButtonState({ view: "server", channel: { kind: "text" }, count: 0 }), { visible: true, count: 0, title: "Threads" });
  assert.equal(threadsButtonState({ view: "server", channel: { kind: "announcement" }, count: 1 }).title, "1 active thread");
  assert.equal(threadsButtonState({ view: "server", channel: { kind: "text" }, count: 3 }).title, "3 active threads");
  assert.equal(threadsButtonState({ view: "server", channel: { kind: "voice" }, count: 3 }).visible, false);
  assert.equal(threadsButtonState({ view: "dm", channel: { kind: "text" }, count: 3 }).visible, false);
  assert.equal(threadsButtonState({}).visible, false);
});
