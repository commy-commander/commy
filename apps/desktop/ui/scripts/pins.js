const MAX_PINS_PER_CHANNEL = 50;

function holdsMessages(channel) {
  return Boolean(channel) && (channel.kind === "text" || channel.kind === "announcement" || channel.kind === "forum");
}

function sortPins(pins) {
  return [...(pins || [])].sort((a, b) => Date.parse(b.at || 0) - Date.parse(a.at || 0));
}

function groupPinsByChannel(pins) {
  const groups = new Map();
  for (const pin of sortPins(pins)) {
    if (!groups.has(pin.channel)) groups.set(pin.channel, { channel: pin.channel, channel_name: pin.channel_name, pins: [] });
    groups.get(pin.channel).pins.push(pin);
  }
  return [...groups.values()];
}

function pinButtonState({ view, channel, count }) {
  const visible = view === "server" && holdsMessages(channel);
  const n = Number.isInteger(count) && count > 0 ? count : 0;
  return {
    visible,
    count: n,
    title: n === 0 ? "Pinned Messages" : n === 1 ? "1 pinned message" : `${n} pinned messages`,
  };
}

const PIN_SEEN_PIECE = "pins.seen.";

function pinTime(pin) {
  const at = Date.parse(pin && pin.at);
  return Number.isFinite(at) ? at : 0;
}

function newestPinTime(pins) {
  return (pins || []).reduce((newest, pin) => Math.max(newest, pinTime(pin)), 0);
}

function unseenPinIds(pins, seen, mark = 0) {
  const known = new Set(seen || []);
  const seenUpTo = Number(mark) || 0;
  return (pins || [])
    .filter(pin => pin && pin.message && pin.message.id && !known.has(pin.message.id))
    .filter(pin => seenUpTo <= 0 || pinTime(pin) > seenUpTo)
    .map(pin => pin.message.id);
}

function pinSeenMarks(pieces) {
  const marks = {};
  for (const [key, value] of Object.entries(pieces || {})) {
    if (!key.startsWith(PIN_SEEN_PIECE)) continue;
    const channel = key.slice(PIN_SEEN_PIECE.length);
    const at = Number(value);
    if (channel && Number.isFinite(at) && at > 0) marks[channel] = at;
  }
  return marks;
}

function pinMarkPieces(marks) {
  const pieces = {};
  for (const [channel, value] of Object.entries(marks || {})) {
    const at = Number(value);
    if (channel && Number.isFinite(at) && at > 0) pieces[`${PIN_SEEN_PIECE}${channel}`] = at;
  }
  return pieces;
}

function mergePinMarks(local, incoming) {
  const merged = { ...(local || {}) };
  for (const [channel, at] of Object.entries(incoming || {})) {
    if (!(Number(merged[channel]) >= at)) merged[channel] = at;
  }
  return merged;
}

function pinMarkChange(marks, channel, pins) {
  const newest = newestPinTime(pins);
  if (!channel || newest <= (Number((marks || {})[channel]) || 0)) return null;
  return { [`${PIN_SEEN_PIECE}${channel}`]: newest };
}

function pinMarksToPublish(local, synced) {
  const known = pinSeenMarks(synced);
  return pinMarkPieces(Object.fromEntries(Object.entries(local || {}).filter(([channel, at]) => !(known[channel] >= Number(at)))));
}

function pinBadgeText(unseen) {
  const n = Number(unseen) || 0;
  if (n <= 0) return "";
  return n > 99 ? "99+" : String(n);
}

function pinTitleWithNew(title, unseen) {
  const n = Number(unseen) || 0;
  return n > 0 && title !== "Pinned Messages" ? `${title} (${n} new)` : title;
}

function canPinIn(server, channel) {
  return Boolean(server && server.can_pin) && holdsMessages(channel);
}

function pinRoomLeft(count) {
  return (Number(count) || 0) < MAX_PINS_PER_CHANNEL;
}

function composerLock({ view, server, channel }) {
  if (view !== "server" || !channel || channel.kind !== "announcement") return null;
  if (server && server.can_post_announcements) return null;
  return {
    title: "Only announcers post here",
    text: "Members allowed to post announcements write in this channel. Everyone can read and react.",
  };
}

function pinExcerpt(message, plain) {
  const raw = plain(message && message.content ? message.content : "");
  const text = raw.replace(/(\*\*|__|~~|`)/g, "").replace(/^#{1,6}\s+/gm, "").trim();
  if (text) return text;
  const attachments = (message && message.attachments) || [];
  if (attachments.length) return attachments.some(a => a.is_image) ? "Sent an image" : "Sent an attachment";
  return "";
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { MAX_PINS_PER_CHANNEL, holdsMessages, sortPins, groupPinsByChannel, pinButtonState, PIN_SEEN_PIECE, newestPinTime, unseenPinIds, pinSeenMarks, pinMarkPieces, mergePinMarks, pinMarkChange, pinMarksToPublish, pinBadgeText, pinTitleWithNew, canPinIn, pinRoomLeft, composerLock, pinExcerpt };
}
