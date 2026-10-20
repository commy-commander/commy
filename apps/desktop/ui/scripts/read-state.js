function messageTime(message) {
  const at = message && message.created_at;
  const time = typeof at === "number" ? at : Date.parse(at);
  return Number.isFinite(time) ? time : 0;
}

function latestMark(messages, now = Date.now()) {
  if (!messages || !messages.length) return null;
  const last = messages[messages.length - 1];
  return { id: last.id, at: messageTime(last), set: now };
}

function markBefore(messages, index, now = Date.now()) {
  if (!messages || index < 0 || index >= messages.length) return null;
  if (index === 0) return { id: null, at: messageTime(messages[0]) - 1, set: now, back: true };
  const previous = messages[index - 1];
  return { id: previous.id, at: messageTime(previous), set: now, back: true };
}

function markOrder(mark) {
  return [Number(mark.at) || 0, String(mark.id || ""), Number(mark.set) || 0];
}

function compareMarks(a, b) {
  const [left, right] = a.back || b.back
    ? [[Number(a.set) || 0, ...markOrder(a)], [Number(b.set) || 0, ...markOrder(b)]]
    : [markOrder(a), markOrder(b)];
  for (let i = 0; i < left.length; i++) {
    if (left[i] !== right[i]) return left[i] > right[i] ? 1 : -1;
  }
  return 0;
}

function newerMark(a, b) {
  if (!a) return b || null;
  if (!b) return a;
  return compareMarks(a, b) >= 0 ? a : b;
}

function mergeReadMaps(a, b) {
  const merged = {};
  for (const source of [a || {}, b || {}]) {
    for (const [channel, mark] of Object.entries(source)) {
      if (!mark || typeof mark !== "object") continue;
      merged[channel] = newerMark(merged[channel], mark);
    }
  }
  return merged;
}

function markAdvances(current, next) {
  if (!next) return false;
  if (!current) return true;
  if ((Number(next.at) || 0) !== (Number(current.at) || 0)) return (Number(next.at) || 0) > (Number(current.at) || 0);
  return String(next.id || "") !== String(current.id || "") && String(next.id || "") > String(current.id || "");
}

function firstAfterMark(messages, mark) {
  if (!messages || !messages.length || !mark) return -1;
  const anchored = mark.id ? messages.findIndex(m => m.id === mark.id) : -1;
  if (anchored >= 0) return anchored + 1 < messages.length ? anchored + 1 : -1;
  const at = Number(mark.at) || 0;
  return messages.findIndex(m => messageTime(m) > at);
}

function countsAsNew(message, me, skip) {
  if (me && message.author_id === me) return false;
  return !(skip && skip(message));
}

function newLineIndex(messages, mark, me, skip) {
  const start = firstAfterMark(messages, mark);
  if (start < 0) return -1;
  for (let i = start; i < messages.length; i++) {
    if (countsAsNew(messages[i], me, skip)) return i;
  }
  return -1;
}

function unreadAfter(messages, mark, me, skip) {
  const start = firstAfterMark(messages, mark);
  if (start < 0) return 0;
  let count = 0;
  for (let i = start; i < messages.length; i++) {
    if (countsAsNew(messages[i], me, skip)) count++;
  }
  return count;
}

function lineFor(line, channelId) {
  if (!line || !channelId || line.channel !== channelId) return null;
  return line.mark || null;
}

function changedMarks(before, after) {
  const changed = {};
  for (const [channel, mark] of Object.entries(after || {})) {
    const old = before ? before[channel] : null;
    if (!old || newerMark(old, mark) !== old) changed[channel] = mark;
  }
  return changed;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { messageTime, latestMark, markBefore, newerMark, mergeReadMaps, markAdvances, firstAfterMark, newLineIndex, unreadAfter, lineFor, changedMarks };
}
