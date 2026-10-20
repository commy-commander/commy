import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { latestMark, markBefore, newerMark, mergeReadMaps, markAdvances, firstAfterMark, newLineIndex, unreadAfter, lineFor, changedMarks } =
  require("../scripts/read-state.js");

const T0 = Date.parse("2026-10-20T10:00:00Z");
const msg = (id, author, minutes) => ({ id, author_id: author, created_at: new Date(T0 + minutes * 60e3).toISOString() });
const messages = [msg("a", "nova", 0), msg("b", "ferris", 5), msg("c", "me", 10), msg("d", "nova", 15), msg("e", "nova", 20)];

test("the latest mark points at the last message, whoever wrote it", () => {
  assert.deepEqual(latestMark(messages, 99), { id: "e", at: T0 + 20 * 60e3, set: 99 });
  assert.equal(latestMark([], 99), null);
  assert.equal(latestMark(null, 99), null);
});

test("marking a message unread puts the mark just before it", () => {
  assert.deepEqual(markBefore(messages, 3, 7), { id: "c", at: T0 + 10 * 60e3, set: 7, back: true });
  assert.deepEqual(markBefore(messages, 0, 7), { id: null, at: T0 - 1, set: 7, back: true });
  assert.equal(markBefore(messages, 5, 7), null);
  assert.equal(markBefore(messages, -1, 7), null);
});

test("plain reads merge to the furthest position, so a device that lags behind cannot pull the mark back", () => {
  const far = { id: "e", at: T0 + 20 * 60e3, set: 100 };
  const stale = { id: "c", at: T0 + 10 * 60e3, set: 200 };
  assert.equal(newerMark(far, stale), far);
  assert.equal(newerMark(stale, far), far);
  assert.equal(newerMark(null, far), far);
  assert.equal(newerMark(far, null), far);
  const same = { id: "x", at: 5, set: 100 };
  assert.equal(newerMark(same, { id: "x", at: 5, set: 101 }).set, 101, "same position: the later write");
});

test("marking unread is deliberate, so it wins by write time and a later read wins back the same way", () => {
  const read = { id: "e", at: T0 + 20 * 60e3, set: 100 };
  const unread = { id: "c", at: T0 + 10 * 60e3, set: 200, back: true };
  assert.equal(newerMark(read, unread), unread);
  assert.equal(newerMark(unread, read), unread);
  const readAgain = { id: "e", at: T0 + 20 * 60e3, set: 300 };
  assert.equal(newerMark(unread, readAgain), readAgain);
  assert.equal(newerMark(readAgain, unread), readAgain);
});

test("read maps merge per channel and keep channels only one side knows", () => {
  const mine = { c1: { id: "a", at: 10, set: 1 }, c2: { id: "b", at: 20, set: 5 } };
  const theirs = { c2: { id: "z", at: 30, set: 4 }, c3: { id: "q", at: 1, set: 2 } };
  assert.deepEqual(mergeReadMaps(mine, theirs), { c1: mine.c1, c2: theirs.c2, c3: theirs.c3 });
  assert.deepEqual(mergeReadMaps(theirs, mine), { c1: mine.c1, c2: theirs.c2, c3: theirs.c3 });
  assert.deepEqual(mergeReadMaps(null, { c1: "junk", c2: mine.c2 }), { c2: mine.c2 });
});

test("reading only ever moves the mark forward", () => {
  assert.equal(markAdvances(null, { id: "a", at: 1 }), true);
  assert.equal(markAdvances({ id: "a", at: 1 }, { id: "b", at: 2 }), true);
  assert.equal(markAdvances({ id: "b", at: 2 }, { id: "a", at: 1 }), false);
  assert.equal(markAdvances({ id: "b", at: 2 }, { id: "b", at: 2 }), false);
  assert.equal(markAdvances({ id: "b", at: 2 }, null), false);
});

test("the first message after the mark is found by id, or by time when the marked message is gone", () => {
  assert.equal(firstAfterMark(messages, { id: "b", at: T0 + 5 * 60e3 }), 2);
  assert.equal(firstAfterMark(messages, { id: "gone", at: T0 + 5 * 60e3 }), 2);
  assert.equal(firstAfterMark(messages, { id: null, at: T0 - 1 }), 0);
  assert.equal(firstAfterMark(messages, { id: "e", at: T0 + 20 * 60e3 }), -1);
  assert.equal(firstAfterMark(messages, null), -1);
  assert.equal(firstAfterMark([], { id: "a", at: 0 }), -1);
});

test("the new-messages line sits above the first unread message that is not mine", () => {
  assert.equal(newLineIndex(messages, { id: "b", at: T0 + 5 * 60e3 }, "me"), 3, "my own message right after the mark is skipped");
  assert.equal(newLineIndex(messages, { id: "a", at: T0 }, "me"), 1);
  assert.equal(newLineIndex(messages, { id: "e", at: T0 + 20 * 60e3 }, "me"), -1);
  assert.equal(newLineIndex(messages, null, "me"), -1, "no mark means nothing is known to be new");
  assert.equal(newLineIndex([msg("a", "nova", 0), msg("b", "me", 1)], { id: "a", at: T0 }, "me"), -1);
  const blocked = m => m.author_id === "nova";
  assert.equal(newLineIndex(messages, { id: "b", at: T0 + 5 * 60e3 }, "me", blocked), -1, "blocked people never make a line");
  assert.equal(newLineIndex(messages, { id: "a", at: T0 }, "me", blocked), 1);
});

test("badge counts leave out my own messages", () => {
  assert.equal(unreadAfter(messages, { id: "a", at: T0 }, "me"), 3);
  assert.equal(unreadAfter(messages, { id: "b", at: T0 + 5 * 60e3 }, "me"), 2);
  assert.equal(unreadAfter(messages, { id: "e", at: T0 + 20 * 60e3 }, "me"), 0);
  assert.equal(unreadAfter(messages, null, "me"), 0);
  assert.equal(unreadAfter(messages, { id: "a", at: T0 }, "me", m => m.author_id === "nova"), 1);
});

test("the frozen line belongs to one channel only", () => {
  const mark = { id: "a", at: 1, set: 1 };
  assert.equal(lineFor({ channel: "c1", mark }, "c1"), mark);
  assert.equal(lineFor({ channel: "c1", mark }, "c2"), null);
  assert.equal(lineFor({ channel: "c1", mark: null }, "c1"), null);
  assert.equal(lineFor(null, "c1"), null);
});

test("a sync reports only the channels whose mark moved", () => {
  const before = { c1: { id: "a", at: 1, set: 1 }, c2: { id: "b", at: 2, set: 2 } };
  const after = { c1: { id: "a", at: 1, set: 1 }, c2: { id: "c", at: 3, set: 3 }, c3: { id: "d", at: 4, set: 4 } };
  assert.deepEqual(changedMarks(before, after), { c2: after.c2, c3: after.c3 });
  assert.deepEqual(changedMarks(before, before), {});
});
