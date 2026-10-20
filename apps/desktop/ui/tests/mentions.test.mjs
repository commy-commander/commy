import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  UNKNOWN_ROLE_MENTION, namesMentioned, mentionsEveryone, isEveryoneMention, roleMentionToken, roleMentions, roleById,
  memberCanMentionEveryone, roleMentionAllowed, mentionableRoles, messageMentionsMe, encodeRoleMentions, decodeRoleMentions,
  splitRoleMentions,
} = require("../scripts/mentions.js");

const MODS = "11111111-2222-3333-4444-555555555555";
const RAID = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
const server = {
  owner: "owner",
  roles: [
    { id: MODS, name: "Moderator", color: "3ba55c", permissions: ["manage_messages"], mentionable: false },
    { id: RAID, name: "Raid Team", color: "9b7bff", permissions: [], mentionable: true },
  ],
  members: [
    { id: "owner", name: "sam", roles: [] },
    { id: "mod", name: "nova", roles: [MODS] },
    { id: "raider", name: "ferris", roles: [RAID] },
    { id: "plain", name: "tidy", roles: [] },
    { id: "herald", name: "echo", roles: [], can_mention_everyone: true },
  ],
};

test("role tokens parse once each and ignore junk", () => {
  const text = `go ${roleMentionToken(RAID)} and ${roleMentionToken(MODS)} and ${roleMentionToken(RAID.toUpperCase())} <@&nope> <@&`;
  assert.deepEqual(roleMentions(text), [RAID, MODS]);
  assert.deepEqual(roleMentions("plain @Raid Team"), []);
  assert.equal(roleMentionToken(RAID), `<@&${RAID}>`);
  assert.deepEqual(splitRoleMentions(`hi ${roleMentionToken(RAID)}!`), [{ text: "hi " }, { role: RAID }, { text: "!" }]);
  assert.deepEqual(splitRoleMentions("nothing"), [{ text: "nothing" }]);
});

test("names and everyone match whole words only", () => {
  assert.equal(namesMentioned("hey @Sam!", ["sam"]), true);
  assert.equal(namesMentioned("hey @samantha", ["sam"]), false);
  assert.equal(namesMentioned("mail sam@example.org", ["sam"]), false);
  assert.equal(namesMentioned("", ["sam"]), false);
  assert.equal(namesMentioned("hi", []), false);
  assert.equal(mentionsEveryone("@everyone up"), true);
  assert.equal(mentionsEveryone("ping @HERE"), true);
  assert.equal(mentionsEveryone("@everyone_else"), false);
  assert.equal(isEveryoneMention("Here"), true);
  assert.equal(isEveryoneMention("sam"), false);
});

test("the owner, admins, members holding the permission and flagged members may mention everyone", () => {
  assert.equal(memberCanMentionEveryone(server, "owner"), true);
  assert.equal(memberCanMentionEveryone(server, "plain"), false);
  assert.equal(memberCanMentionEveryone(server, "herald"), true);
  assert.equal(memberCanMentionEveryone(server, "stranger"), false);
  const withRole = { ...server, roles: [...server.roles, { id: "x", name: "Herald", permissions: ["mention_everyone"] }] };
  withRole.members = [...server.members, { id: "h2", name: "h2", roles: ["x"] }, { id: "a", name: "a", roles: ["y"] }];
  withRole.roles.push({ id: "y", name: "Admin", permissions: ["administrator"] });
  assert.equal(memberCanMentionEveryone(withRole, "h2"), true);
  assert.equal(memberCanMentionEveryone(withRole, "a"), true);
});

test("a closed role is only mentionable by members who may mention everyone", () => {
  const mods = roleById(server, MODS);
  const raid = roleById(server, RAID.toUpperCase());
  assert.equal(roleMentionAllowed(server, "plain", mods), false);
  assert.equal(roleMentionAllowed(server, "owner", mods), true);
  assert.equal(roleMentionAllowed(server, "plain", raid), true);
  assert.equal(roleMentionAllowed(server, "plain", null), false);
  assert.deepEqual(mentionableRoles(server, "plain").map(r => r.name), ["Raid Team"]);
  assert.deepEqual(mentionableRoles(server, "owner").map(r => r.name), ["Moderator", "Raid Team"]);
});

test("a message reaches me by name, by a permitted everyone, or by a role I hold", () => {
  const me = { id: "mod", names: ["nova", null] };
  assert.equal(messageMentionsMe({ content: "hi @nova", server, authorId: "plain", me }), true);
  assert.equal(messageMentionsMe({ content: "@everyone hi", server, authorId: "owner", me }), true);
  assert.equal(messageMentionsMe({ content: "@everyone hi", server, authorId: "plain", me }), false);
  assert.equal(messageMentionsMe({ content: `${roleMentionToken(MODS)} help`, server, authorId: "owner", me }), true);
  assert.equal(messageMentionsMe({ content: `${roleMentionToken(MODS)} help`, server, authorId: "plain", me }), false);
  assert.equal(messageMentionsMe({ content: `${roleMentionToken(RAID)} go`, server, authorId: "plain", me }), false);
  const raider = { id: "raider", names: ["ferris"] };
  assert.equal(messageMentionsMe({ content: `${roleMentionToken(RAID)} go`, server, authorId: "plain", me: raider }), true);
  assert.equal(messageMentionsMe({ content: "", server, authorId: "owner", me }), false);
  assert.equal(messageMentionsMe({ content: "@everyone", server: null, authorId: "owner", me }), false);
});

test("typed role names become tokens on the way out and names on the way back", () => {
  const roles = server.roles;
  assert.equal(encodeRoleMentions("hey @Raid Team, and @moderator", roles), `hey ${roleMentionToken(RAID)}, and ${roleMentionToken(MODS)}`);
  assert.equal(encodeRoleMentions("@Raid Teammates", roles), "@Raid Teammates");
  assert.equal(encodeRoleMentions("mail raid@Raid Team", roles), "mail raid@Raid Team");
  assert.equal(encodeRoleMentions("@Moderator", roles, ["moderator"]), "@Moderator", "a member with that name wins");
  assert.equal(encodeRoleMentions("plain", []), "plain");
  assert.equal(decodeRoleMentions(`ask ${roleMentionToken(MODS)} or <@&00000000-0000-0000-0000-000000000000>`, roles), `ask @Moderator or ${UNKNOWN_ROLE_MENTION}`);
});
