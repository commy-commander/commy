const TIMEOUT_CHOICES = [
  { id: "60s", label: "60 seconds", seconds: 60 },
  { id: "5m", label: "5 minutes", seconds: 5 * 60 },
  { id: "10m", label: "10 minutes", seconds: 10 * 60 },
  { id: "1h", label: "1 hour", seconds: 60 * 60 },
  { id: "1d", label: "1 day", seconds: 24 * 60 * 60 },
  { id: "1w", label: "1 week", seconds: 7 * 24 * 60 * 60 },
];

const MAX_TIMEOUT_SECS = 28 * 24 * 60 * 60;

function timeoutRemainingSecs(untilMs, nowMs = Date.now()) {
  const until = Number(untilMs);
  if (!Number.isFinite(until)) return 0;
  return Math.max(0, Math.ceil((until - nowMs) / 1000));
}

function timeoutActive(untilMs, nowMs = Date.now()) {
  return timeoutRemainingSecs(untilMs, nowMs) > 0;
}

function remainingWords(secs) {
  const total = Math.max(0, Math.floor(Number(secs) || 0));
  if (total < 60) return `${total}s`;
  const units = [["w", 7 * 86400], ["d", 86400], ["h", 3600], ["m", 60], ["s", 1]];
  const parts = [];
  let left = total;
  for (const [suffix, size] of units) {
    const n = Math.floor(left / size);
    if (n > 0 || (parts.length && suffix === "s" && total < 3600)) {
      parts.push(`${n}${suffix}`);
      left -= n * size;
    }
    if (parts.length === 2) break;
  }
  return parts.join(" ");
}

function timeoutLeftText(untilMs, nowMs = Date.now()) {
  return remainingWords(timeoutRemainingSecs(untilMs, nowMs));
}

function timeoutBadgeTitle(untilMs, nowMs = Date.now()) {
  return `Timed out for another ${timeoutLeftText(untilMs, nowMs)}`;
}

function timeoutLock({ view, server, nowMs = Date.now() }) {
  if (view !== "server" || !server || !timeoutActive(server.my_timeout_until, nowMs)) return null;
  return {
    title: "You are timed out",
    text: "You can read and listen, but not send messages, react, edit or talk for another ",
    until: Number(server.my_timeout_until),
  };
}

function customTimeoutSeconds(minutesText) {
  const trimmed = String(minutesText ?? "").trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const minutes = Number(trimmed);
  if (minutes < 1 || minutes * 60 > MAX_TIMEOUT_SECS) return null;
  return minutes * 60;
}

function canTimeOut(server, member, meId) {
  if (!server || !member || !server.can_time_out) return false;
  if (member.id === server.owner || (meId && member.id === meId)) return false;
  return (server.members || []).some(m => m.id === member.id);
}

function timeoutActionLabel(member, nowMs = Date.now()) {
  return timeoutActive(member && member.timeout_until, nowMs) ? "Remove timeout" : "Time out";
}

function timeoutSummary(name, seconds) {
  const choice = TIMEOUT_CHOICES.find(c => c.seconds === seconds);
  const length = choice ? choice.label : `${Math.round(seconds / 60)} minutes`;
  return `${name} is timed out for ${length}`;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    TIMEOUT_CHOICES, MAX_TIMEOUT_SECS, timeoutRemainingSecs, timeoutActive, remainingWords, timeoutLeftText,
    timeoutBadgeTitle, timeoutLock, customTimeoutSeconds, canTimeOut, timeoutActionLabel, timeoutSummary,
  };
}
