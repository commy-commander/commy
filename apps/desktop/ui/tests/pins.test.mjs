import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { MAX_PINS_PER_CHANNEL, holdsMessages, sortPins, groupPinsByChannel, pinButtonState, PIN_SEEN_PIECE, newestPinTime, unseenPinIds, pinSeenMarks, pinMarkPieces, mergePinMarks, pinMarkChange, pinMarksToPublish, pinBadgeText, pinTitleWithNew, canPinIn, pinRoomLeft, composerLock, pinExcerpt } =
  require("../scripts/pins.js");

const plain = s => s;

test("the pin button shows in text and announcement channels only, with its count", () => {
  assert.deepEqual(pinButtonState({ view: "server", channel: { kind: "text" }, count: 0 }), { visible: true, count: 0, title: "Pinned Messages" });
  assert.deepEqual(pinButtonState({ view: "server", channel: { kind: "announcement" }, count: 1 }), { visible: true, count: 1, title: "1 pinned message" });
  assert.equal(pinButtonState({ view: "server", channel: { kind: "text" }, count: 7 }).title, "7 pinned messages");
  assert.equal(pinButtonState({ view: "server", channel: { kind: "voice" }, count: 2 }).visible, false);
  assert.equal(pinButtonState({ view: "dm", channel: { kind: "text" }, count: 2 }).visible, false);
  assert.equal(pinButtonState({ view: "server", channel: null, count: 2 }).visible, false);
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

test("the badge counts pins not seen yet and clears once they are", () => {
  const pins = [{ message: { id: "1" } }, { message: { id: "2" } }, { message: { id: "3" } }];
  assert.deepEqual(unseenPinIds(pins, undefined), ["1", "2", "3"]);
  assert.deepEqual(unseenPinIds(pins, ["1", "3"]), ["2"]);
  assert.deepEqual(unseenPinIds(pins, ["1", "2", "3", "gone"]), []);
  assert.deepEqual(unseenPinIds(undefined, ["1"]), []);
  assert.equal(pinBadgeText(0), "");
  assert.equal(pinBadgeText(4), "4");
  assert.equal(pinBadgeText(120), "99+");
  assert.equal(pinTitleWithNew("3 pinned messages", 2), "3 pinned messages (2 new)");
  assert.equal(pinTitleWithNew("3 pinned messages", 0), "3 pinned messages");
  assert.equal(pinTitleWithNew("Pinned Messages", 0), "Pinned Messages");
});

test("pins seen on another device clear the badge here", () => {
  const pins = [
    { message: { id: "1" }, at: "2026-11-12T10:00:00Z" },
    { message: { id: "2" }, at: "2026-11-12T11:00:00Z" },
    { message: { id: "3" }, at: "2026-11-12T12:00:00Z" },
  ];
  const upTo = Date.parse("2026-11-12T11:00:00Z");
  assert.deepEqual(unseenPinIds(pins, [], upTo), ["3"]);
  assert.deepEqual(unseenPinIds(pins, ["3"], upTo), []);
  assert.deepEqual(unseenPinIds(pins, [], newestPinTime(pins)), []);
  assert.deepEqual(unseenPinIds(pins, [], 0), ["1", "2", "3"]);
  assert.equal(newestPinTime(pins), Date.parse("2026-11-12T12:00:00Z"));
  assert.equal(newestPinTime([]), 0);
});

test("viewing pins makes one synced setting per channel that only moves forward", () => {
  const pins = [{ message: { id: "1" }, at: "2026-11-12T10:00:00Z" }];
  const at = Date.parse("2026-11-12T10:00:00Z");
  assert.equal(PIN_SEEN_PIECE, "pins.seen.");
  assert.deepEqual(pinMarkChange({}, "c1", pins), { "pins.seen.c1": at });
  assert.equal(pinMarkChange({ c1: at }, "c1", pins), null);
  assert.equal(pinMarkChange({ c1: at + 1 }, "c1", pins), null);
  assert.equal(pinMarkChange({}, "c1", []), null);
  assert.equal(pinMarkChange({}, "", pins), null);
});

test("synced pin settings read back into marks and merge to the newest", () => {
  const pieces = { "pins.seen.c1": 5, "pins.seen.c2": null, "notify.level": "all", "pins.seen.": 3, "pins.seen.c3": "junk" };
  assert.deepEqual(pinSeenMarks(pieces), { c1: 5 });
  assert.deepEqual(pinMarkPieces({ c1: 5, c2: 0 }), { "pins.seen.c1": 5 });
  assert.deepEqual(mergePinMarks({ c1: 9, c2: 1 }, { c1: 4, c2: 7, c3: 2 }), { c1: 9, c2: 7, c3: 2 });
  assert.deepEqual(pinMarksToPublish({ c1: 9, c2: 1, c3: 4 }, { "pins.seen.c1": 3, "pins.seen.c2": 1 }), { "pins.seen.c1": 9, "pins.seen.c3": 4 });
  assert.deepEqual(pinMarksToPublish({}, {}), {});
});
