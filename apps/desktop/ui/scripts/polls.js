const MIN_POLL_OPTIONS = 2;
const MAX_POLL_OPTIONS = 10;
const MAX_POLL_QUESTION_CHARS = 300;
const MAX_POLL_OPTION_CHARS = 100;

const POLL_DURATIONS = [
  { hours: 1, label: "1 hour" },
  { hours: 8, label: "8 hours" },
  { hours: 24, label: "1 day" },
  { hours: 72, label: "3 days" },
  { hours: 168, label: "1 week" },
  { hours: null, label: "No end" },
];

const DEFAULT_POLL_HOURS = 24;

function squashPollText(text) {
  return String(text || "").split(/\s+/).filter(Boolean).join(" ");
}

function cleanPollOptions(values) {
  return (values || []).map(squashPollText).filter(Boolean);
}

function pollDraftProblem({ question, options }) {
  const q = squashPollText(question);
  const answers = cleanPollOptions(options);
  if (!q) return "Ask a question first";
  if (q.length > MAX_POLL_QUESTION_CHARS) return `Questions are up to ${MAX_POLL_QUESTION_CHARS} characters`;
  if (answers.length < MIN_POLL_OPTIONS) return `Give at least ${MIN_POLL_OPTIONS} answers`;
  if (answers.length > MAX_POLL_OPTIONS) return `A poll holds up to ${MAX_POLL_OPTIONS} answers`;
  if (answers.some(a => a.length > MAX_POLL_OPTION_CHARS)) return `Answers are up to ${MAX_POLL_OPTION_CHARS} characters`;
  const seen = new Set();
  for (const answer of answers) {
    const key = answer.toLowerCase();
    if (seen.has(key)) return "Two answers say the same thing";
    seen.add(key);
  }
  return null;
}

function pollIsClosed(poll, now = Date.now()) {
  if (!poll) return false;
  if (poll.closed) return true;
  const at = poll.closes_at ? Date.parse(poll.closes_at) : NaN;
  return Number.isFinite(at) && at <= now;
}

function pollShares(poll) {
  const total = Math.max(0, Number(poll && poll.total) || 0);
  return ((poll && poll.options) || []).map(option => {
    const votes = Math.max(0, Number(option.votes) || 0);
    const percent = total === 0 ? 0 : Math.min(100, Math.round((votes / total) * 100));
    return { votes, percent };
  });
}

function pollVoteCount(total) {
  const n = Math.max(0, Number(total) || 0);
  if (n === 0) return "No votes yet";
  return n === 1 ? "1 vote" : `${n} votes`;
}

function pollTimeLeft(closesAt, now = Date.now()) {
  const at = closesAt ? Date.parse(closesAt) : NaN;
  if (!Number.isFinite(at)) return "";
  const left = at - now;
  if (left <= 0) return "ended";
  const minutes = Math.round(left / 60e3);
  if (minutes < 1) return "ending now";
  if (minutes < 60) return `ends in ${minutes} ${minutes === 1 ? "minute" : "minutes"}`;
  const hours = Math.round(left / 3600e3);
  if (hours < 24) return `ends in ${hours} ${hours === 1 ? "hour" : "hours"}`;
  const days = Math.round(left / 86400e3);
  return `ends in ${days} ${days === 1 ? "day" : "days"}`;
}

function pollFooter(poll, now = Date.now()) {
  const votes = pollVoteCount(poll && poll.total);
  if (pollIsClosed(poll, now)) return `Final results · ${votes}`;
  const left = pollTimeLeft(poll && poll.closes_at, now);
  return left ? `${votes} · ${left}` : votes;
}

function pollMyChoices(poll) {
  return ((poll && poll.options) || []).map((o, i) => (o.mine ? i : -1)).filter(i => i >= 0);
}

function nextPollChoices(poll, index) {
  const mine = pollMyChoices(poll);
  if (poll && poll.multi) {
    return mine.includes(index) ? mine.filter(i => i !== index) : [...mine, index].sort((a, b) => a - b);
  }
  return mine.includes(index) ? [] : [index];
}

function canClosePoll({ poll, own, canManage, now = Date.now() }) {
  return Boolean(poll) && !pollIsClosed(poll, now) && Boolean(own || canManage);
}

function pollHint(poll, now = Date.now()) {
  if (pollIsClosed(poll, now)) return "";
  return poll && poll.multi ? "Pick as many as you like" : "Pick one";
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    MIN_POLL_OPTIONS, MAX_POLL_OPTIONS, MAX_POLL_QUESTION_CHARS, MAX_POLL_OPTION_CHARS, POLL_DURATIONS, DEFAULT_POLL_HOURS,
    squashPollText, cleanPollOptions, pollDraftProblem, pollIsClosed, pollShares, pollVoteCount, pollTimeLeft, pollFooter,
    pollMyChoices, nextPollChoices, canClosePoll, pollHint,
  };
}
