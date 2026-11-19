import "./i18n-global.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { inviteAsksTrust, inviteWarningText, listedHomeLabel, listedHomeOptions, signInServerNote } = require("../scripts/server-listing.js");

const invite = over => ({ host: "commy.chat", account_server_host: "commy.chat", trusted: true, community_host: null, ...over });
const host = over => ({ origin: "https://rooms.example.org", host: "rooms.example.org", listed: null, ask: true, ...over });

test("joining asks for trust only when the community host is unlisted and new", () => {
  assert.equal(inviteAsksTrust(invite()), false);
  assert.equal(inviteAsksTrust(invite({ community_host: host({ ask: false }) })), false);
  assert.equal(inviteAsksTrust(invite({ community_host: host() })), true);
  assert.equal(inviteAsksTrust(null), false);
});

test("the warning names the community host and keeps the account server note", () => {
  assert.equal(inviteWarningText(invite()), "");
  const hostOnly = inviteWarningText(invite({ community_host: host() }));
  assert.match(hostOnly, /^This community keeps its messages on rooms\.example\.org, which is not on Commy's list/);
  assert.match(hostOnly, /cannot read the messages/);
  const both = inviteWarningText(invite({ trusted: false, host: "other.example.net", community_host: host() }));
  assert.match(both, /^This invite comes from other\.example\.net, not your account server \(commy\.chat\)\./);
  assert.match(both, /rooms\.example\.org/);
});

test("listed homes show their name and address", () => {
  assert.equal(listedHomeLabel({ name: "Example Chat", host: "chat.example.org" }), "Example Chat (chat.example.org)");
  assert.equal(listedHomeLabel({ name: "chat.example.org", host: "chat.example.org" }), "chat.example.org");
  const options = listedHomeOptions([{ origin: "https://a.example.org", name: "A", host: "a.example.org" }], "https://a.example.org");
  assert.deepEqual(options, [{ value: "https://a.example.org", label: "A (a.example.org)", selected: true }]);
  assert.deepEqual(listedHomeOptions(undefined, null), []);
});

test("the note under the picker matches the choice", () => {
  const account = { server: "https://mine.example.org", server_listed: false, listed_homes: [{ origin: "https://a.example.org" }] };
  assert.equal(signInServerNote("default", account), "signin.server.note");
  assert.equal(signInServerNote("default", { ...account, server_impostor: true }), "signin.server.unverified");
  assert.equal(signInServerNote("https://a.example.org", account), "signin.server.listed");
  assert.equal(signInServerNote("https://mine.example.org", account), "signin.server.warning");
  assert.equal(signInServerNote("https://mine.example.org", { ...account, server_listed: true }), "signin.server.listed");
  assert.equal(signInServerNote("custom", account), "signin.server.warning");
});
