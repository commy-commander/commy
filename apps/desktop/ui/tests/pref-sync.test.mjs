import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { prefPieces, prefsWithPieces, changedPieces, syncedValues, startupPrefs } = require("../scripts/pref-sync.js");

const prefs = {
  level: "mentions",
  friendOnline: true,
  servers: { s1: "none", s2: "all" },
  muted: { s3: true },
  channels: { s1: { c1: { mute: 1700000000 } } },
};

test("settings split into one piece per server, channel group and switch", () => {
  assert.deepEqual(prefPieces(prefs), {
    "notify.level": "mentions",
    "notify.friendOnline": true,
    "notify.servers.s1": "none",
    "notify.servers.s2": "all",
    "notify.muted.s3": true,
    "notify.channels.s1": { c1: { mute: 1700000000 } },
  });
  assert.deepEqual(prefsWithPieces({}, prefPieces(prefs)), prefs);
  assert.deepEqual(prefPieces(null), {});
});

test("only what changed is sent, and removals are sent as empty", () => {
  const next = { ...prefs, level: "all", servers: { s1: "none" }, muted: { s3: true, s4: true } };
  assert.deepEqual(changedPieces(prefs, next), {
    "notify.level": "all",
    "notify.servers.s2": null,
    "notify.muted.s4": true,
  });
  assert.deepEqual(changedPieces(prefs, prefs), {});
});

test("pieces from another device land without touching the rest", () => {
  const applied = prefsWithPieces(prefs, { "notify.servers.s2": null, "notify.muted.s5": true, "notify.friendOnline": null, "other.key": 1, "notify.": 2, "notify..x": 3 });
  assert.deepEqual(applied.servers, { s1: "none" });
  assert.deepEqual(applied.muted, { s3: true, s5: true });
  assert.equal("friendOnline" in applied, false);
  assert.equal(applied.level, "mentions");
  assert.equal("other" in applied, false);
  assert.deepEqual(prefs.servers, { s1: "none", s2: "all" }, "the original is left alone");
});

test("on start the synced settings win and local-only ones are offered to the other devices", () => {
  const synced = { "notify.level": { value: "none", at: 9 }, "notify.servers.s2": { value: null, at: 9 } };
  const { prefs: merged, publish } = startupPrefs(prefs, synced);
  assert.equal(merged.level, "none");
  assert.deepEqual(merged.servers, { s1: "none" });
  assert.deepEqual(Object.keys(publish).sort(), ["notify.channels.s1", "notify.friendOnline", "notify.muted.s3", "notify.servers.s1"]);
  assert.deepEqual(syncedValues({ a: {}, b: { value: 0 } }), { a: null, b: 0 });
  assert.deepEqual(startupPrefs({}, null), { prefs: {}, publish: {} });
});
