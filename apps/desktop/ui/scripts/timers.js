// Elapsed-time text for the clocks next to people: how long someone has
// been in a voice channel or in a game. Pure functions of a start time in
// unix seconds and "now" in milliseconds, kept free of the DOM so they
// can be tested with node.

// "12:34", or "1:02:03" past an hour. Never negative.
function elapsedClock(sinceSecs, nowMs = Date.now()) {
  const total = Math.max(0, Math.floor(nowMs / 1000) - Number(sinceSecs || 0));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = h ? String(m).padStart(2, "0") : String(m);
  return `${h ? `${h}:` : ""}${mm}:${String(s).padStart(2, "0")}`;
}

// "1h 12m", "12m", or "under a minute".
function elapsedWords(sinceSecs, nowMs = Date.now()) {
  const total = Math.max(0, Math.floor(nowMs / 1000) - Number(sinceSecs || 0));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  if (!h && !m) return "under a minute";
  return h ? `${h}h ${m}m` : `${m}m`;
}

if (typeof module !== "undefined") module.exports = { elapsedClock, elapsedWords };
