import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  MIN_POLL_OPTIONS, MAX_POLL_OPTIONS, MAX_POLL_QUESTION_CHARS, MAX_POLL_OPTION_CHARS, POLL_DURATIONS, DEFAULT_POLL_HOURS,
  cleanPollOptions, pollDraftProblem, pollIsClosed, pollShares, pollVoteCount, pollTimeLeft, pollFooter, pollMyChoices,
  nextPollChoices, canClosePoll, pollHint,
} = require("../scripts/polls.js");

const NOW = Date.parse("2026-10-20T12:00:00Z");
const iso = ms => new Date(NOW + ms).toISOString();

const poll = (over = {}) => ({
  question: "Raid night?",
  options: [{ text: "Friday", votes: 3, mine: true }, { text: "Saturday", votes: 1, mine: false }, { text: "Sunday", votes: 0, mine: false }],
  multi: false,
  total: 4,
  closed: false,
  closes_at: iso(3 * 3600e3),
  ...over,
});

test("the duration list runs from an hour to a week and ends with no end", () => {
  assert.deepEqual(POLL_DURATIONS.map(d => d.label), ["1 hour", "8 hours", "1 day", "3 days", "1 week", "No end"]);
  assert.deepEqual(POLL_DURATIONS.map(d => d.hours), [1, 8, 24, 72, 168, null]);
  assert.ok(POLL_DURATIONS.some(d => d.hours === DEFAULT_POLL_HOURS));
});

test("option rows are trimmed, squashed and empty ones dropped", () => {
  assert.deepEqual(cleanPollOptions(["  Friday ", "", "Sat\n\nurday", null, "  "]), ["Friday", "Sat urday"]);
  assert.deepEqual(cleanPollOptions(undefined), []);
});

test("a draft needs a question and two to ten distinct answers within the limits", () => {
  assert.equal(pollDraftProblem({ question: "Raid night?", options: ["Fri", "Sat"] }), null);
  assert.equal(pollDraftProblem({ question: "  ", options: ["Fri", "Sat"] }), "Ask a question first");
  assert.equal(pollDraftProblem({ question: "Raid night?", options: ["Fri", " "] }), `Give at least ${MIN_POLL_OPTIONS} answers`);
  const many = Array.from({ length: MAX_POLL_OPTIONS + 1 }, (_, i) => `opt ${i}`);
  assert.equal(pollDraftProblem({ question: "q", options: many }), `A poll holds up to ${MAX_POLL_OPTIONS} answers`);
  assert.equal(pollDraftProblem({ question: "q", options: many.slice(0, MAX_POLL_OPTIONS) }), null);
  assert.equal(pollDraftProblem({ question: "q".repeat(MAX_POLL_QUESTION_CHARS + 1), options: ["a", "b"] }), `Questions are up to ${MAX_POLL_QUESTION_CHARS} characters`);
  assert.equal(pollDraftProblem({ question: "q", options: ["a", "b".repeat(MAX_POLL_OPTION_CHARS + 1)] }), `Answers are up to ${MAX_POLL_OPTION_CHARS} characters`);
  assert.equal(pollDraftProblem({ question: "q", options: ["Friday", "friday "] }), "Two answers say the same thing");
});

test("a poll is closed once flagged or past its closing time", () => {
  assert.equal(pollIsClosed(poll(), NOW), false);
  assert.equal(pollIsClosed(poll({ closed: true }), NOW), true);
  assert.equal(pollIsClosed(poll({ closes_at: iso(-1) }), NOW), true);
  assert.equal(pollIsClosed(poll({ closes_at: iso(0) }), NOW), true);
  assert.equal(pollIsClosed(poll({ closes_at: null }), NOW), false);
  assert.equal(pollIsClosed(poll({ closes_at: "garbage" }), NOW), false);
  assert.equal(pollIsClosed(null, NOW), false);
});

test("shares are the part of voters who picked each answer, so multi polls can sum past 100", () => {
  assert.deepEqual(pollShares(poll()), [{ votes: 3, percent: 75 }, { votes: 1, percent: 25 }, { votes: 0, percent: 0 }]);
  const multi = poll({ multi: true, total: 2, options: [{ text: "a", votes: 2 }, { text: "b", votes: 1 }] });
  assert.deepEqual(pollShares(multi), [{ votes: 2, percent: 100 }, { votes: 1, percent: 50 }]);
  assert.deepEqual(pollShares(poll({ total: 0, options: [{ text: "a", votes: 0 }] })), [{ votes: 0, percent: 0 }]);
  assert.deepEqual(pollShares(poll({ total: 3, options: [{ text: "a", votes: 1 }] })), [{ votes: 1, percent: 33 }]);
  assert.deepEqual(pollShares(null), []);
});

test("the footer counts votes and says when the poll ends or that it is final", () => {
  assert.equal(pollVoteCount(0), "No votes yet");
  assert.equal(pollVoteCount(1), "1 vote");
  assert.equal(pollVoteCount(12), "12 votes");
  assert.equal(pollTimeLeft(iso(3 * 3600e3), NOW), "ends in 3 hours");
  assert.equal(pollTimeLeft(iso(3600e3), NOW), "ends in 1 hour");
  assert.equal(pollTimeLeft(iso(45 * 60e3), NOW), "ends in 45 minutes");
  assert.equal(pollTimeLeft(iso(20e3), NOW), "ending now");
  assert.equal(pollTimeLeft(iso(2 * 86400e3), NOW), "ends in 2 days");
  assert.equal(pollTimeLeft(iso(7 * 86400e3), NOW), "ends in 7 days");
  assert.equal(pollTimeLeft(iso(-5e3), NOW), "ended");
  assert.equal(pollTimeLeft(null, NOW), "");
  assert.equal(pollFooter(poll({ total: 12 }), NOW), "12 votes · ends in 3 hours");
  assert.equal(pollFooter(poll({ total: 12, closes_at: null }), NOW), "12 votes");
  assert.equal(pollFooter(poll({ total: 0, closes_at: null }), NOW), "No votes yet");
  assert.equal(pollFooter(poll({ total: 12, closed: true }), NOW), "Final results · 12 votes");
  assert.equal(pollFooter(poll({ total: 1, closes_at: iso(-1) }), NOW), "Final results · 1 vote");
});

test("a click picks one answer, picks it again to retract, and toggles in multi polls", () => {
  assert.deepEqual(pollMyChoices(poll()), [0]);
  assert.deepEqual(nextPollChoices(poll(), 1), [1]);
  assert.deepEqual(nextPollChoices(poll(), 0), []);
  const multi = poll({ multi: true, options: [{ text: "a", mine: true }, { text: "b", mine: false }, { text: "c", mine: true }] });
  assert.deepEqual(pollMyChoices(multi), [0, 2]);
  assert.deepEqual(nextPollChoices(multi, 1), [0, 1, 2]);
  assert.deepEqual(nextPollChoices(multi, 2), [0]);
  assert.deepEqual(nextPollChoices(poll({ options: [] }), 0), [0]);
});

test("the author or a moderator can close an open poll, nobody a finished one", () => {
  assert.equal(canClosePoll({ poll: poll(), own: true, canManage: false, now: NOW }), true);
  assert.equal(canClosePoll({ poll: poll(), own: false, canManage: true, now: NOW }), true);
  assert.equal(canClosePoll({ poll: poll(), own: false, canManage: false, now: NOW }), false);
  assert.equal(canClosePoll({ poll: poll({ closed: true }), own: true, canManage: true, now: NOW }), false);
  assert.equal(canClosePoll({ poll: poll({ closes_at: iso(-1) }), own: true, canManage: true, now: NOW }), false);
  assert.equal(canClosePoll({ poll: null, own: true, canManage: true, now: NOW }), false);
});

test("the hint tells people how many answers they can pick until the poll closes", () => {
  assert.equal(pollHint(poll(), NOW), "Pick one");
  assert.equal(pollHint(poll({ multi: true }), NOW), "Pick as many as you like");
  assert.equal(pollHint(poll({ closed: true }), NOW), "");
});
