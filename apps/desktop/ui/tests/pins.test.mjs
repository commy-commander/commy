// Run with: node --test "apps/desktop/ui/tests/**/*.test.mjs"
import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { MAX_PINS_PER_CHANNEL, holdsMessages, sortPins, groupPinsByChannel, pinButtonState, canPinIn, pinRoomLeft, composerLock, pinExcerpt } =
  require("../scripts/pins.js");

const plain = s => s;

test("the pin button shows in text and announcement channels only, with its count", () => {
  assert.deepEqual(pinButtonState({ view: "server", channel: { kind: "text" }, count: 0 }), { visible: true, count: 0, title: "Pinned Messages" });
  assert.deepEqual(pinButtonState({ view: "server", channel: { kind: "announcement" }, count: 1 }), { visible: true, count: 1, title: "1 pinned message" });
  assert.equal(pinButtonState({ view: "server", channel: { kind: "text" }, count: 7 }).title, "7 pinned messages");
  assert.equal(pinButtonState({ view: "server", channel: { kind: "voice" }, count: 2 }).visible, false);
  assert.equal(pinButtonState({ view: "dm", channel: { kind: "text" }, count: 2 }).visible, false);
  assert.equal(pinButtonState({ view: "server", channel: null, count: 2 }).visible, false);
  // A missing or odd count reads as none.
  assert.equal(pinButtonState({ view: "server", channel: { kind: "text" }, count: undefined }).count, 0);
  assert.equal(pinButtonState({ view: "server", channel: { kind: "text" }, count: -3 }).count, 0);
});

test("the panel lists pins newest first and groups them by channel for the server view", () => {
  const pins = [
    { channel: "a", channel_name: "general", at: "2026-10-16T10:00:00Z", message: { id: "1" } },
    { channel: "b", channel_name: "news", at: "2026-10-16T12:00:00Z", message: { id: "2" } },
    { channel: "a", channel_name: "general", at: "2026-10-16T11:00:00Z", message: { id: "3" } },
  ];
  assert.deepEqual(sortPins(pins).map(p => p.message.id), ["2", "3", "1"]);
  const groups = groupPinsByChannel(pins);
  assert.deepEqual(groups.map(g => [g.channel_name, g.pins.map(p => p.message.id)]), [["news", ["2"]], ["general", ["3", "1"]]]);
  assert.deepEqual(sortPins(undefined), []);
});

test("pinning needs the permission and a channel that holds messages", () => {
  assert.equal(canPinIn({ can_pin: true }, { kind: "text" }), true);
  assert.equal(canPinIn({ can_pin: true }, { kind: "announcement" }), true);
  assert.equal(canPinIn({ can_pin: false }, { kind: "text" }), false);
  assert.equal(canPinIn({ can_pin: true }, { kind: "voice" }), false);
  assert.equal(canPinIn(null, { kind: "text" }), false);
  assert.equal(holdsMessages({ kind: "category" }), false);
  assert.equal(pinRoomLeft(MAX_PINS_PER_CHANNEL - 1), true);
  assert.equal(pinRoomLeft(MAX_PINS_PER_CHANNEL), false);
  assert.equal(pinRoomLeft(undefined), true);
});

test("the composer gives way to a notice in announcement channels for members who cannot post", () => {
  const server = { can_post_announcements: false };
  const lock = composerLock({ view: "server", server, channel: { kind: "announcement" } });
  assert.equal(lock.title, "Only announcers post here");
  assert.match(lock.text, /read and react/);
  assert.equal(composerLock({ view: "server", server: { can_post_announcements: true }, channel: { kind: "announcement" } }), null);
  assert.equal(composerLock({ view: "server", server, channel: { kind: "text" } }), null);
  assert.equal(composerLock({ view: "dm", server, channel: { kind: "announcement" } }), null);
  assert.equal(composerLock({ view: "server", server, channel: null }), null);
});

test("a pin's excerpt is its text, else what it attached", () => {
  assert.equal(pinExcerpt({ content: "**Patch day** at 6" }, plain), "Patch day at 6");
  assert.equal(pinExcerpt({ content: "## Heads up\n`restart` needed" }, plain), "Heads up\nrestart needed");
  assert.equal(pinExcerpt({ content: "", attachments: [{ is_image: true }] }, plain), "Sent an image");
  assert.equal(pinExcerpt({ content: "  ", attachments: [{ is_image: false }] }, plain), "Sent an attachment");
  assert.equal(pinExcerpt({ content: "" }, plain), "");
  assert.equal(pinExcerpt(null, plain), "");
});
