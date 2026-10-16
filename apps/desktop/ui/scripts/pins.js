// Pinned messages and announcement channels: the decisions behind the
// pin button, the pins panel and the composer, kept free of DOM and
// backend so they run in the app and in `node --test` alike (see
// ui/tests/pins.test.mjs).

const MAX_PINS_PER_CHANNEL = 50;

// Text-like channels: the ones that hold messages and so can hold pins.
function holdsMessages(channel) {
  return Boolean(channel) && (channel.kind === "text" || channel.kind === "announcement");
}

// Newest pin first; ties keep their order.
function sortPins(pins) {
  return [...(pins || [])].sort((a, b) => Date.parse(b.at || 0) - Date.parse(a.at || 0));
}

// Pins grouped by channel in the order their newest pin falls, each
// group newest first, for the server-wide view.
function groupPinsByChannel(pins) {
  const groups = new Map();
  for (const pin of sortPins(pins)) {
    if (!groups.has(pin.channel)) groups.set(pin.channel, { channel: pin.channel, channel_name: pin.channel_name, pins: [] });
    groups.get(pin.channel).pins.push(pin);
  }
  return [...groups.values()];
}

// The pin button in the chat header: shown in text and announcement
// channels, its tooltip counting the pins.
function pinButtonState({ view, channel, count }) {
  const visible = view === "server" && holdsMessages(channel);
  const n = Number.isInteger(count) && count > 0 ? count : 0;
  return {
    visible,
    count: n,
    title: n === 0 ? "Pinned Messages" : n === 1 ? "1 pinned message" : `${n} pinned messages`,
  };
}

// Whether the viewer may pin here: the server grants it and the channel
// holds messages.
function canPinIn(server, channel) {
  return Boolean(server && server.can_pin) && holdsMessages(channel);
}

// Whether one more pin fits in the channel.
function pinRoomLeft(count) {
  return (Number(count) || 0) < MAX_PINS_PER_CHANNEL;
}

// What replaces the composer when the viewer cannot post here: only in
// announcement channels, for members without the permission.
function composerLock({ view, server, channel }) {
  if (view !== "server" || !channel || channel.kind !== "announcement") return null;
  if (server && server.can_post_announcements) return null;
  return {
    title: "Only announcers post here",
    text: "Members allowed to post announcements write in this channel. Everyone can read and react.",
  };
}

// One line of a pinned message for the list, without markdown marks.
function pinExcerpt(message, plain) {
  const raw = plain(message && message.content ? message.content : "");
  const text = raw.replace(/(\*\*|__|~~|`)/g, "").replace(/^#{1,6}\s+/gm, "").trim();
  if (text) return text;
  const attachments = (message && message.attachments) || [];
  if (attachments.length) return attachments.some(a => a.is_image) ? "Sent an image" : "Sent an attachment";
  return "";
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { MAX_PINS_PER_CHANNEL, holdsMessages, sortPins, groupPinsByChannel, pinButtonState, canPinIn, pinRoomLeft, composerLock, pinExcerpt };
}
