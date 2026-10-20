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

test("a group conversation lists its people but is never a mutual server", () => {
  const servers = [
    { id: "s1", name: "One", members: [member("a", "me"), member("b", "pal")] },
    { id: "g1", name: "pal, other", group: true, members: [member("a", "me"), member("b", "pal"), member("c", "other")] },
  ];
  const people = indexPeople(servers, "a");
  assert.deepEqual(mutualServers(people, "b").map(s => s.id), ["s1"]);
  assert.equal(people.has("c"), true);
  assert.deepEqual(mutualServers(people, "c"), []);
});

test("a game keeps its own cover and start time, taken from the server that reported it", () => {
  const servers = [
    { id: "s1", name: "One", members: [member("b", "pal")] },
    { id: "s2", name: "Two", members: [member("b", "pal", "online", { activity: "Playing Celeste", activity_cover: "co3byy", activity_since: 100 })] },
    { id: "s3", name: "Three", members: [member("b", "pal", "online", { activity: "Playing Portal 2", activity_cover: "co1rs4", activity_since: 200 })] },
  ];
  const pal = indexPeople(servers, "a").get("b");
  assert.equal(pal.activity, "Playing Celeste");
  assert.equal(pal.activity_cover, "co3byy");
  assert.equal(pal.activity_since, 100);
});
