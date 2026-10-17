function elapsedParts(sinceSecs, nowMs) {
  const total = Math.max(0, Math.floor(nowMs / 1000) - Number(sinceSecs || 0));
  return { h: Math.floor(total / 3600), m: Math.floor((total % 3600) / 60), s: total % 60 };
}

function elapsedClock(sinceSecs, nowMs = Date.now()) {
  const { h, m, s } = elapsedParts(sinceSecs, nowMs);
  const mm = h ? String(m).padStart(2, "0") : String(m);
  return `${h ? `${h}:` : ""}${mm}:${String(s).padStart(2, "0")}`;
}

function elapsedWords(sinceSecs, nowMs = Date.now()) {
  const { h, m } = elapsedParts(sinceSecs, nowMs);
  if (!h && !m) return "under a minute";
  return h ? `${h}h ${m}m` : `${m}m`;
}

if (typeof module !== "undefined") module.exports = { elapsedClock, elapsedWords };
