import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { indexPeople, mutualServers } = require("../scripts/people.js");

const member = (id, name, presence = "online", extra = {}) => ({ id, name, presence, custom_status: null, status_emoji: null, activity: null, ...extra });

test("someone in the same server as you shows that server as mutual, and you are never in common with yourself", () => {
  const servers = [{ id: "s1", name: "Bench", members: [member("a", "bencha"), member("b", "benchb")] }];
  const people = indexPeople(servers, "a");
  assert.deepEqual(mutualServers(people, "b"), [{ id: "s1", name: "Bench" }]);
  assert.equal(people.has("a"), false);
});

test("servers are folded per person, in order, without repeats, keeping the liveliest presence and the first status", () => {
  const servers = [
    { id: "s1", name: "One", members: [member("a", "me"), member("b", "pal", "offline")] },
    { id: "s2", name: "Two", members: [member("b", "pal", "idle", { custom_status: "afk" }), member("c", "other")] },
    { id: "s2", name: "Two", members: [member("b", "pal")] },
  ];
  const people = indexPeople(servers, "a");
  assert.deepEqual(mutualServers(people, "b").map(s => s.id), ["s1", "s2"]);
  assert.deepEqual(mutualServers(people, "c").map(s => s.id), ["s2"]);
  assert.deepEqual(mutualServers(people, "nobody"), []);
  assert.equal(people.get("b").presence, "idle");
  assert.equal(people.get("b").custom_status, "afk");
});

test("an empty server list means no one in common", () => {
  assert.equal(indexPeople([], "a").size, 0);
  assert.deepEqual(mutualServers(new Map(), "b"), []);
});
