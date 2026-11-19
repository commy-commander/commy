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
  { hours: null, get label() { return t("ui.no-end"); } },
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
  if (!q) return t("ui.ask-a-question-first");
  if (q.length > MAX_POLL_QUESTION_CHARS) return t("ui.questions-are-up-to-characters", { MAX_POLL_QUESTION_CHARS });
  if (answers.length < MIN_POLL_OPTIONS) return t("ui.give-at-least-answers", { MIN_POLL_OPTIONS });
  if (answers.length > MAX_POLL_OPTIONS) return t("ui.a-poll-holds-up-to-answers", { MAX_POLL_OPTIONS });
  if (answers.some(a => a.length > MAX_POLL_OPTION_CHARS)) return t("ui.answers-are-up-to-characters", { MAX_POLL_OPTION_CHARS });
  const seen = new Set();
  for (const answer of answers) {
    const key = answer.toLowerCase();
    if (seen.has(key)) return t("ui.two-answers-say-the-same-thing");
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
  if (n === 0) return t("ui.no-votes-yet");
  return t(n === 1 ? "ui.n.votes.one" : "ui.n.votes.many", { count: n });
}

function pollTimeLeft(closesAt, now = Date.now()) {
  const at = closesAt ? Date.parse(closesAt) : NaN;
  if (!Number.isFinite(at)) return "";
  const left = at - now;
  if (left <= 0) return t("ui.poll-ended");
  const minutes = Math.round(left / 60e3);
  if (minutes < 1) return t("ui.ending-now");
  if (minutes < 60) return t(minutes === 1 ? "ui.ends-in-minutes.one" : "ui.ends-in-minutes.many", { count: minutes });
  const hours = Math.round(left / 3600e3);
  if (hours < 24) return t(hours === 1 ? "ui.ends-in-hours.one" : "ui.ends-in-hours.many", { count: hours });
  const days = Math.round(left / 86400e3);
  return t(days === 1 ? "ui.ends-in-days.one" : "ui.ends-in-days.many", { count: days });
}

function pollFooter(poll, now = Date.now()) {
  const votes = pollVoteCount(poll && poll.total);
  if (pollIsClosed(poll, now)) return t("ui.final-results", { votes });
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
  return poll && poll.multi ? t("ui.pick-as-many-as-you-like") : t("ui.pick-one");
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    MIN_POLL_OPTIONS, MAX_POLL_OPTIONS, MAX_POLL_QUESTION_CHARS, MAX_POLL_OPTION_CHARS, POLL_DURATIONS, DEFAULT_POLL_HOURS,
    squashPollText, cleanPollOptions, pollDraftProblem, pollIsClosed, pollShares, pollVoteCount, pollTimeLeft, pollFooter,
    pollMyChoices, nextPollChoices, canClosePoll, pollHint,
  };
}
