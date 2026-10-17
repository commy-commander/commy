const MAX_PINS_PER_CHANNEL = 50;

function holdsMessages(channel) {
  return Boolean(channel) && (channel.kind === "text" || channel.kind === "announcement");
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

function unseenPinIds(pins, seen) {
  const known = new Set(seen || []);
  return (pins || []).map(p => p && p.message && p.message.id).filter(id => id && !known.has(id));
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
  module.exports = { MAX_PINS_PER_CHANNEL, holdsMessages, sortPins, groupPinsByChannel, pinButtonState, unseenPinIds, pinBadgeText, pinTitleWithNew, canPinIn, pinRoomLeft, composerLock, pinExcerpt };
}
