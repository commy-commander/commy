const SYNCED_PREF_PREFIX = "notify.";

function isPlainObject(value) {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function prefPieces(prefs) {
  const pieces = {};
  for (const [key, value] of Object.entries(prefs || {})) {
    if (value === undefined) continue;
    if (isPlainObject(value)) {
      for (const [child, inner] of Object.entries(value)) {
        if (inner !== undefined) pieces[`${SYNCED_PREF_PREFIX}${key}.${child}`] = inner;
      }
    } else {
      pieces[`${SYNCED_PREF_PREFIX}${key}`] = value;
    }
  }
  return pieces;
}

function prefsWithPieces(prefs, pieces) {
  const next = { ...(prefs || {}) };
  for (const [key, value] of Object.entries(pieces || {})) {
    if (!key.startsWith(SYNCED_PREF_PREFIX)) continue;
    const rest = key.slice(SYNCED_PREF_PREFIX.length);
    const dot = rest.indexOf(".");
    if (!rest || dot === 0) continue;
    if (dot === -1) {
      if (value === null || value === undefined) delete next[rest];
      else next[rest] = value;
      continue;
    }
    const top = rest.slice(0, dot);
    const child = rest.slice(dot + 1);
    const holder = isPlainObject(next[top]) ? { ...next[top] } : {};
    if (value === null || value === undefined) delete holder[child];
    else holder[child] = value;
    next[top] = holder;
  }
  return next;
}

function changedPieces(before, after) {
  const was = prefPieces(before);
  const now = prefPieces(after);
  const changes = {};
  for (const key of new Set([...Object.keys(was), ...Object.keys(now)])) {
    if (JSON.stringify(was[key]) !== JSON.stringify(now[key])) changes[key] = key in now ? now[key] : null;
  }
  return changes;
}

function syncedValues(synced) {
  return Object.fromEntries(Object.entries(synced || {}).map(([key, piece]) => [key, piece && piece.value !== undefined ? piece.value : null]));
}

function startupPrefs(local, synced) {
  const known = synced || {};
  const prefs = prefsWithPieces(local, syncedValues(known));
  const publish = Object.fromEntries(Object.entries(prefPieces(local)).filter(([key]) => !(key in known)));
  return { prefs, publish };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { SYNCED_PREF_PREFIX, prefPieces, prefsWithPieces, changedPieces, syncedValues, startupPrefs };
}
