import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { AUDIT_FILTERS, auditGroup, describeAudit, auditParts, auditText, filterAudit, auditActors } = require("../scripts/audit-log.js");

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const rustKinds = () => {
  const source = readFileSync(join(repo, "crates", "commy-core", "src", "audit.rs"), "utf8");
  const body = /pub enum AuditKind \{([\s\S]*?)\}/.exec(source)[1];
  return [...body.matchAll(/^\s*([A-Z][A-Za-z]*),/gm)].map(m => m[1].replace(/([a-z])([A-Z])/g, "$1_$2").toLowerCase());
};

const entry = (kind, fields = {}) => ({ at: 1, by: "u1", by_name: "Mia", kind, target: null, label: null, detail: null, ...fields });

test("every kind the core records reads as a sentence and sits in a filter group", () => {
  const kinds = rustKinds();
  assert.ok(kinds.length > 30);
  const groups = new Set(AUDIT_FILTERS.map(([id]) => id));
  for (const kind of kinds) {
    const text = auditText(entry(kind, { label: "x", detail: "1" }));
    assert.notEqual(text, "Mia made a change", kind);
    assert.ok(groups.has(auditGroup(kind)), kind);
  }
});

test("sentences name the actor, the target and the change", () => {
  assert.equal(auditText(entry("member_ban", { label: "Theo" })), "Mia banned Theo");
  assert.equal(auditText(entry("channel_rename", { label: "general", detail: "lobby" })), "Mia renamed the channel general to lobby");
  assert.equal(auditText(entry("server_rename", { label: "Old", detail: "New" })), "Mia renamed the server from Old to New");
  assert.equal(auditText(entry("role_assign", { label: "Sam", detail: "mods" })), "Mia gave Sam the mods role");
  assert.equal(auditText(entry("role_unassign", { label: "Sam", detail: "mods" })), "Mia took the mods role from Sam");
  assert.equal(auditText(entry("channel_create", { label: "hangout", detail: "voice" })), "Mia created the voice channel hangout");
  assert.equal(auditText(entry("channel_retention", { label: "news", detail: "7" })), "Mia set messages in news to disappear after 7 days");
  assert.equal(auditText(entry("channel_retention", { label: "news" })), "Mia turned off disappearing messages in news");
  assert.equal(auditText(entry("channel_limit", { label: "Duo", detail: "1" })), "Mia set a limit of 1 person in Duo");
  assert.equal(auditText(entry("channel_limit", { label: "Duo", detail: "0" })), "Mia removed the people limit in Duo");
  assert.equal(auditText(entry("sticker_add", { label: "party" })), "Mia added the sticker party");
  assert.equal(auditText(entry("emoticon_remove", { label: "shrug" })), "Mia removed the emoticon :shrug");
  assert.equal(auditText(entry("expression_sharing", { detail: "stickers and emoticons" })), "Mia let members use this server's stickers and emoticons in other servers and DMs");
  assert.equal(auditText(entry("expression_sharing", { detail: "" })), "Mia stopped sharing this server's stickers and emoticons");
  assert.equal(auditText(entry("voice_moderate", { label: "Sam", detail: "muted" })), "Mia server-muted Sam");
  assert.equal(auditText(entry("voice_moderate", { label: "Sam", detail: "cleared" })), "Mia lifted the server mute on Sam");
  assert.equal(auditText(entry("message_delete", { label: "Nova", detail: "general" })), "Mia deleted a message by Nova in general");
  assert.equal(auditText(entry("emoji_add", { label: "wave" })), "Mia added the emoji :wave:");
  assert.equal(auditText(entry("server_public", { detail: "off" })), "Mia made the server private");
  assert.equal(auditText(entry("afk_timeout", { detail: "15" })), "Mia set the server AFK timer to 15 minutes");
  assert.equal(auditText(entry("invite_create", { label: "pixel-pioneers" })), "Mia created the invite pixel-pioneers");
  assert.equal(auditText(entry("invite_revoke", { label: "KQ7M3N4P", detail: "Theo" })), "Mia revoked the invite KQ7M3N4P made by Theo");
  assert.equal(auditText(entry("invite_background", { detail: "removed" })), "Mia removed the invite background");
});

test("missing names fall back to plain words instead of blanks", () => {
  assert.equal(auditText(entry("member_kick")), "Mia kicked a member");
  assert.equal(auditText(entry("channel_delete")), "Mia deleted a channel");
  assert.equal(auditText({ ...entry("role_delete"), by_name: "" }), "Someone deleted a role");
  assert.equal(auditText(entry("something_new")), "Mia made a change");
});

test("names are marked so the page can set them apart", () => {
  const parts = auditParts(entry("member_ban", { label: "Theo" }));
  assert.deepEqual(parts, [{ text: "Mia", strong: true }, { text: " " }, { text: "banned " }, { text: "Theo", strong: true }]);
  assert.ok(describeAudit(entry("auto_mod")).every(p => typeof p === "string"));
});

test("filters narrow by group and by who acted, and actors list once each by name", () => {
  const log = [
    entry("member_ban", { by: "a", by_name: "Zed" }),
    entry("channel_delete", { by: "b", by_name: "Ann" }),
    entry("role_assign", { by: "a", by_name: "Zed" }),
  ];
  assert.equal(filterAudit(log).length, 3);
  assert.deepEqual(filterAudit(log, { group: "members" }).map(e => e.kind), ["member_ban", "role_assign"]);
  assert.deepEqual(filterAudit(log, { by: "b" }).map(e => e.kind), ["channel_delete"]);
  assert.deepEqual(filterAudit(log, { group: "channels", by: "a" }), []);
  assert.deepEqual(auditActors(log), [{ id: "b", name: "Ann" }, { id: "a", name: "Zed" }]);
  assert.deepEqual(filterAudit(null), []);
});
