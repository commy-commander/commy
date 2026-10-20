const SLOW_MODE_OPTIONS = [0, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200, 21600];

function slowModeLabel(seconds) {
  const s = Number(seconds) || 0;
  if (s <= 0) return "Off";
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.round(s / 60)}m`;
  return `${Math.round(s / 3600)}h`;
}

function slowModeOptionText(seconds) {
  const s = Number(seconds) || 0;
  if (s <= 0) return "Off";
  if (s < 60) return `${s} seconds`;
  if (s < 3600) return s === 60 ? "1 minute" : `${Math.round(s / 60)} minutes`;
  return s === 3600 ? "1 hour" : `${Math.round(s / 3600)} hours`;
}

function slowModeWait(sentAt, seconds, now) {
  const s = Number(seconds) || 0;
  if (s <= 0 || !sentAt) return 0;
  const left = Math.ceil((sentAt + s * 1000 - now) / 1000);
  return left > 0 ? left : 0;
}

function slowModeState({ channel, canBypass, sentAt, now }) {
  const seconds = channel ? Number(channel.slow_mode) || 0 : 0;
  if (!seconds || canBypass) return { shown: false, waiting: 0, seconds };
  const waiting = slowModeWait(sentAt, seconds, now);
  return { shown: true, waiting, seconds, text: waiting ? `${waiting}s` : slowModeLabel(seconds) };
}

function slowModeWaitFromError(text) {
  const m = /send again in (?:(\d+)h)?\s*(?:(\d+)m)?\s*(?:(\d+)s)?/.exec(String(text || ""));
  if (!m || (!m[1] && !m[2] && !m[3])) return 0;
  return (Number(m[1]) || 0) * 3600 + (Number(m[2]) || 0) * 60 + (Number(m[3]) || 0);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { SLOW_MODE_OPTIONS, slowModeLabel, slowModeOptionText, slowModeWait, slowModeState, slowModeWaitFromError };
}
