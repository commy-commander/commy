import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { accessJoin, accessWho, channelIsPrivate, channelAccessLock, toggleAccessRole, sameAccess, accessSummary } =
  require("../scripts/channel-access.js");

const server = { roles: [{ id: "a", name: "Admins" }, { id: "d", name: "Donators" }, { id: "m", name: "Mods" }] };

test("role lists read as plain English", () => {
  assert.equal(accessJoin([]), "");
  assert.equal(accessJoin(["Mods"]), "Mods");
  assert.equal(accessJoin(["Mods", "Donators"]), "Mods and Donators");
  assert.equal(accessJoin(["Admins", "Mods", "Donators"]), "Admins, Mods and Donators");
  assert.equal(accessWho(server, ["m", "gone"]), "Mods");
  assert.equal(accessWho(server, ["gone"]), "admins");
  assert.equal(accessSummary(server, []), "Everyone");
  assert.equal(accessSummary(server, ["d", "m"]), "Donators and Mods");
});

test("a channel is private once it names roles that can see it", () => {
  assert.equal(channelIsPrivate({ view_roles: [] }), false);
  assert.equal(channelIsPrivate({ view_roles: ["m"] }), true);
  assert.equal(channelIsPrivate({}), false);
  assert.equal(channelIsPrivate(null), false);
});

test("the composer explains why you cannot write in a locked channel", () => {
  const view = "server";
  const unseen = channelAccessLock({ view, server, channel: { kind: "text", view_roles: ["m"], can_view: false } });
  assert.equal(unseen.title, "You can't read this channel");
  assert.match(unseen.text, /Only Mods can read it/);

  const readOnly = channelAccessLock({ view, server, channel: { kind: "text", send_roles: ["m", "d"], can_view: true, can_send: false } });
  assert.equal(readOnly.title, "Only Mods and Donators can send messages here");

  const waiting = channelAccessLock({ view, server, channel: { kind: "text", view_roles: ["m"], can_view: true, can_send: true, key_ready: false } });
  assert.equal(waiting.title, "Getting this channel's key");

  assert.equal(channelAccessLock({ view, server, channel: { kind: "text", view_roles: ["m"], can_view: true, can_send: true, key_ready: true } }), null);
  assert.equal(channelAccessLock({ view, server, channel: { kind: "announcement", can_view: true, can_send: false, send_roles: [] } }), null);
  assert.equal(channelAccessLock({ view: "dm", server, channel: { kind: "text", can_view: false } }), null);
  assert.equal(channelAccessLock({ view, server, channel: { kind: "voice", can_view: false } }), null);
});

test("role picks toggle and compare regardless of order", () => {
  assert.deepEqual(toggleAccessRole([], "m"), ["m"]);
  assert.deepEqual(toggleAccessRole(["m", "d"], "m"), ["d"]);
  assert.deepEqual(toggleAccessRole(undefined, "a"), ["a"]);
  assert.equal(sameAccess(["m", "d"], ["d", "m"]), true);
  assert.equal(sameAccess(["m"], ["m", "d"]), false);
  assert.equal(sameAccess(undefined, []), true);
});
