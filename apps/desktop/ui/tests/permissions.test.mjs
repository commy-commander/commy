import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  PERMISSION_HINTS, EVERYONE_CHAT_ACCESS, RESTRICTION_NAMES, HISTORY_NOTICE, LINK_EMBED_OFF,
  permissionHint, isRestriction, switchOn, namesToSave, grantedNames, everyoneLacks, canEmbedLinks, canReadHistory, canCreateThreads, canSendPolls, canCreateEvents,
  showsEmbeds, threadEntryShown, historyWindow,
} = require("../scripts/permissions.js");

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const read = (...path) => readFileSync(join(repo, ...path), "utf8");
const appSource = read("apps", "desktop", "ui", "scripts", "app.js");
const viewSource = read("crates", "commy-view", "src", "lib.rs");
const rolesSource = read("crates", "commy-view", "src", "roles.rs");

const editable = [...(/pub const EDITABLE_PERMISSIONS[\s\S]*?\];/.exec(viewSource)[0]).matchAll(/\("([a-z_]+)", Permissions::([A-Z_]+)\)/g)].map(m => [m[1], m[2]]);
const labels = [...(/const PERMISSION_LABELS = \[([\s\S]*?)\n\];/.exec(appSource)[1]).matchAll(/\["([a-z_]+)", "([^"]+)"\]/g)].map(m => [m[1], m[2]]);
const grouped = [...(/const PERMISSION_GROUPS = \[([\s\S]*?)\n\];/.exec(appSource)[1]).matchAll(/"([a-z_]+)"/g)].map(m => m[1]).filter(n => n !== n.toUpperCase());

test("the five restrictions take bits 36 to 40 in order and sit in the editor under positive names", () => {
  const core = read("crates", "commy-core", "src", "permissions.rs");
  const names = new Map(editable);
  for (const [name, bit] of [["embed_links", 36], ["read_message_history", 37], ["create_threads", 38], ["send_polls", 39], ["create_events", 40]]) {
    const constant = `NO_${name === "send_polls" ? "SEND_POLLS" : name.toUpperCase()}`;
    assert.ok(core.includes(`pub const ${constant}: Permissions = Permissions(1 << ${bit});`), `${constant} is bit ${bit}`);
    assert.equal(names.get(name), constant, `${name} maps to the inverted bit`);
    assert.ok(isRestriction(name));
  }
  assert.deepEqual(RESTRICTION_NAMES, ["embed_links", "read_message_history", "create_threads", "send_polls", "create_events"]);
  assert.ok(core.includes("pub const RESTRICTIONS: Permissions"));
  assert.ok(!isRestriction("manage_events") && !isRestriction("attach_files"));
  for (const [name, constant] of editable) assert.equal(constant.startsWith("NO_"), isRestriction(name), `${name} is listed as a restriction exactly when its bit is one`);
});

test("switches read positive: a restriction is on when its bit is clear, and saving inverts it back", () => {
  const held = new Set(["manage_messages", "send_polls"]);
  assert.equal(switchOn(held, "manage_messages"), true);
  assert.equal(switchOn(held, "pin_messages"), false);
  assert.equal(switchOn(held, "send_polls"), false, "the bit is set, so the switch is off");
  assert.equal(switchOn(held, "embed_links"), true, "no bit, so allowed");
  assert.equal(switchOn(["send_polls"], "send_polls"), false);
  assert.deepEqual(namesToSave([["manage_messages", true], ["pin_messages", false], ["send_polls", false], ["embed_links", true]]), ["manage_messages", "send_polls"]);
  assert.deepEqual(namesToSave([]), []);
  assert.deepEqual(grantedNames(["manage_messages", "send_polls", "embed_links"]), ["manage_messages"]);
  assert.deepEqual(grantedNames(null), []);
});

test("every editable permission has a label, a one-line hint and a place in the roles form", () => {
  const labelNames = new Map(labels);
  for (const [name] of editable) {
    assert.ok(labelNames.has(name), `${name} has a label`);
    assert.ok(labelNames.get(name).length <= 40, `${name} label is short`);
    const hint = permissionHint(name);
    assert.ok(hint.length > 0 && hint.length <= 90 && !hint.includes("\n"), `${name} has a one-line hint`);
  }
  for (const name of ["embed_links", "read_message_history", "create_threads", "send_polls", "create_events"]) {
    assert.ok(grouped.includes(name), `${name} is in a group`);
  }
  assert.equal(labels.length, editable.length);
  assert.equal(Object.keys(PERMISSION_HINTS).length, editable.length);
  assert.equal(permissionHint("made_up"), "");
  const groupText = /const PERMISSION_GROUPS = \[([\s\S]*?)\n\];/.exec(appSource)[1];
  const chat = /\["Chat", \[([^\]]*)\]/.exec(groupText)[1];
  for (const name of ["embed_links", "read_message_history", "create_threads", "send_polls"]) assert.ok(chat.includes(`"${name}"`), `${name} is a chat permission`);
  assert.ok(/\["Server", \[[^\]]*"create_events", "manage_events"/.test(groupText), "create events sits next to manage events");
});

test("the everyone switches name settings the backend knows and flags the view reports", () => {
  const backend = /fn role_access_permission[\s\S]*?\n\}/.exec(rolesSource)[0];
  const struct = /pub struct ChatRestrictions \{([\s\S]*?)\}/.exec(viewSource)[1];
  const labelNames = new Map(labels);
  assert.equal(EVERYONE_CHAT_ACCESS.length, 5);
  for (const spec of EVERYONE_CHAT_ACCESS) {
    assert.ok(backend.includes(`"${spec.key}" =>`), `${spec.key} is a known access setting`);
    assert.ok(struct.includes(`pub ${spec.field}: bool`), `${spec.field} is reported`);
    assert.ok(labelNames.has(spec.permission));
    assert.ok(spec.title && spec.sub && !spec.sub.includes("\n"));
  }
  const spec = EVERYONE_CHAT_ACCESS[0];
  assert.equal(everyoneLacks(null, spec), false);
  assert.equal(everyoneLacks({}, spec), false);
  assert.equal(everyoneLacks({ chat_restricted: { embed_links: true } }, spec), true);
  assert.equal(everyoneLacks({ chat_restricted: { embed_links: false } }, spec), false);
});

test("the gates read the server view and default to allowed", () => {
  for (const [can, flag] of [[canEmbedLinks, "can_embed_links"], [canReadHistory, "can_read_history"], [canCreateThreads, "can_create_threads"], [canSendPolls, "can_send_polls"]]) {
    assert.equal(can(null), true);
    assert.equal(can({}), true);
    assert.equal(can({ [flag]: true }), true);
    assert.equal(can({ [flag]: false }), false);
  }
  assert.equal(canCreateEvents(null), true);
  assert.equal(canCreateEvents({ can_create_events: false }), false);
  assert.equal(canCreateEvents({ can_create_events: false, can_manage_events: true }), true, "managing events covers creating");
  assert.equal(canCreateEvents({ can_create_events: true, can_manage_events: false }), true);
});

test("messages hide their cards when the author may not embed, and threads stay reachable once they exist", () => {
  assert.equal(showsEmbeds({ embeds: false }), false);
  assert.equal(showsEmbeds({ embeds: true }), true);
  assert.equal(showsEmbeds({}), true, "older views without the flag keep their cards");
  assert.equal(threadEntryShown({ replies: 0, meta: null, canCreate: true }), true);
  assert.equal(threadEntryShown({ replies: 0, meta: null, canCreate: false }), false);
  assert.equal(threadEntryShown({ replies: 2, meta: null, canCreate: false }), true);
  assert.equal(threadEntryShown({ replies: 0, meta: { name: "plans" }, canCreate: false }), true);
  assert.equal(threadEntryShown({}), true);
});

test("without history only messages from after the channel was opened remain", () => {
  const at = ms => new Date(1_700_000_000_000 + ms).toISOString();
  const messages = [{ id: "a", created_at: at(0) }, { id: "b", created_at: at(5_000) }, { id: "c", created_at: at(9_000) }];
  assert.deepEqual(historyWindow(messages, 1_700_000_000_000 + 5_000).map(m => m.id), ["b", "c"]);
  assert.deepEqual(historyWindow(messages, 1_700_000_000_000 + 9_001), []);
  assert.deepEqual(historyWindow(messages, null), [], "a channel never opened shows nothing");
  assert.deepEqual(historyWindow(null, 1), []);
  assert.equal(HISTORY_NOTICE, "You can see new messages only");
  assert.ok(LINK_EMBED_OFF.title && LINK_EMBED_OFF.sub);
});
