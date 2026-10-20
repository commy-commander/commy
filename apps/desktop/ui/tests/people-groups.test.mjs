import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { groupMembers, inviteCandidates } = require("../scripts/people.js");

const roles = [
  { id: "admin", name: "Administrators", position: 5, hoist: true, color: "ff4d4d" },
  { id: "donor", name: "Donators", position: 3, hoist: true },
  { id: "plain", name: "Raider", position: 2, hoist: false },
];

const member = (id, presence, held) => ({ id, name: id, presence, roles: held });

test("hoisted roles sit above the rest of the online members, highest first", () => {
  const groups = groupMembers([
    member("a", "online", ["donor"]),
    member("b", "online", ["plain"]),
    member("c", "online", ["donor", "admin"]),
  ], roles);
  assert.deepEqual(groups.map(g => [g.title, g.members.map(m => m.id)]), [
    ["Administrators", ["c"]],
    ["Donators", ["a"]],
    ["Online", ["b"]],
  ]);
});

test("offline members stay in one dimmed group whatever their roles", () => {
  const groups = groupMembers([member("a", "offline", ["admin"]), member("b", "invisible", [])], roles);
  assert.equal(groups.length, 1);
  assert.equal(groups[0].title, "Offline");
  assert.equal(groups[0].dim, true);
  assert.equal(groups[0].members.length, 2);
});

test("empty groups are not drawn and no hoisted roles keeps the plain split", () => {
  const groups = groupMembers([member("a", "online", ["admin"])], [{ id: "admin", name: "Administrators", position: 5, hoist: false }]);
  assert.deepEqual(groups.map(g => g.title), ["Online"]);
});

test("invite candidates skip people already in the server and put friends first", () => {
  const people = new Map([
    ["a", { id: "a", name: "Ada", presence: "offline" }],
    ["b", { id: "b", name: "Bo", presence: "online" }],
    ["c", { id: "c", name: "Cy", presence: "online" }],
    ["d", { id: "d", name: "Di", presence: "online" }],
  ]);
  const all = inviteCandidates(people, ["a"], ["d"], "");
  assert.deepEqual(all.map(p => p.id), ["a", "b", "c"]);
  assert.equal(all[0].friend, true);
  assert.deepEqual(inviteCandidates(people, [], [], "  b").map(p => p.id), ["b"]);
});
