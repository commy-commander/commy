import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { statusLine, backLine, subLine, presenceLabel } = require("../scripts/presence.js");

const now = Date.UTC(2026, 9, 16, 12, 0, 0);
const inAnHour = Math.floor(now / 1000) + 3600;

test("the back line shows only for a running Do Not Disturb", () => {
  assert.match(backLine({ presence: "do_not_disturb", presence_until: inAnHour }, now), /^Back at /);
  assert.equal(backLine({ presence: "do_not_disturb", presence_until: Math.floor(now / 1000) - 1 }, now), "");
  assert.equal(backLine({ presence: "do_not_disturb" }, now), "");
  assert.equal(backLine({ presence: "idle", presence_until: inAnHour }, now), "");
  assert.equal(backLine(null, now), "");
});

test("when you are back outranks what you are doing", () => {
  const busy = { presence: "do_not_disturb", presence_until: inAnHour, activity: "Playing Hollow Knight" };
  assert.match(subLine(busy, now), /^Back at .* · Playing Hollow Knight$/);
  const status = { presence: "do_not_disturb", presence_until: inAnHour, custom_status: "in a meeting", status_emoji: "📚" };
  assert.match(subLine(status, now), /^Back at .* · 📚 in a meeting$/);
  assert.equal(subLine({ presence: "online", custom_status: "heads down" }, now), "heads down");
  assert.equal(subLine({ presence: "online" }, now), "");
});

test("status line prefers the activity, then emoji and status", () => {
  assert.equal(statusLine({ activity: "Using Blender", custom_status: "x" }), "Using Blender");
  assert.equal(statusLine({ status_emoji: "🎧", custom_status: "focus" }), "🎧 focus");
  assert.equal(statusLine(null), "");
});

test("presence label carries the back time in lower case", () => {
  assert.match(presenceLabel({ presence: "do_not_disturb", presence_until: inAnHour }, now), /^Do Not Disturb · back at /);
  assert.equal(presenceLabel({ presence: "idle" }, now), "Idle");
  assert.equal(presenceLabel({}, now), "Offline");
});

test("an offline person whose phone gets notifications shows a phone mark", async () => {
  const { onPhone, presenceDot } = require("../scripts/presence.js");
  assert.equal(presenceDot({ presence: "offline", phone: true }), "offline phone");
  assert.equal(presenceDot({ presence: "invisible", phone: true }), "offline phone");
  assert.equal(presenceDot({ presence: "online", phone: true }), "online", "someone at the app shows their real presence");
  assert.equal(presenceDot({ presence: "offline" }), "offline");
  assert.equal(presenceDot(null), "offline");
  assert.equal(onPhone({ presence: "idle", phone: true }), false);
  assert.equal(presenceLabel({ presence: "offline", phone: true }, now), "Offline · gets notifications on phone");
  assert.equal(presenceLabel({ presence: "offline" }, now), "Offline");
});
