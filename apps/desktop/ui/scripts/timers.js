function elapsedClock(sinceSecs, nowMs = Date.now()) {
  const total = Math.max(0, Math.floor(nowMs / 1000) - Number(sinceSecs || 0));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = h ? String(m).padStart(2, "0") : String(m);
  return `${h ? `${h}:` : ""}${mm}:${String(s).padStart(2, "0")}`;
}

function elapsedWords(sinceSecs, nowMs = Date.now()) {
  const total = Math.max(0, Math.floor(nowMs / 1000) - Number(sinceSecs || 0));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  if (!h && !m) return "under a minute";
  return h ? `${h}h ${m}m` : `${m}m`;
}

if (typeof module !== "undefined") module.exports = { elapsedClock, elapsedWords };
