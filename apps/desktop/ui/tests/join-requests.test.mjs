import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  JOIN_NOTE_MAX, joinNoteLeft, joinRequestsLabel, joinWaitStatus, joinWaitMembers, newJoinRequests, joinRequestToast,
} = require("../scripts/join-requests.js");

const ui = join(import.meta.dirname, "..");
const core = readFileSync(join(ui, "..", "..", "..", "crates", "commy-core", "src", "join_request.rs"), "utf8");
const html = readFileSync(join(ui, "index.html"), "utf8");

test("the note limit matches the one the app enforces", () => {
  assert.match(core, new RegExp(`MAX_NOTE_CHARS: usize = ${JOIN_NOTE_MAX};`));
  assert.match(html, new RegExp(`id="join-wait-note"[^>]*maxlength="${JOIN_NOTE_MAX}"`));
  assert.equal(joinNoteLeft("  hi  "), JOIN_NOTE_MAX - 2);
  assert.equal(joinNoteLeft("\u{1f680}"), JOIN_NOTE_MAX - 1);
  assert.equal(joinNoteLeft(null), JOIN_NOTE_MAX);
});

test("the tab counts who is waiting", () => {
  assert.equal(joinRequestsLabel(0), "Join Requests");
  assert.equal(joinRequestsLabel(3), "Join Requests (3)");
});

test("the waiting room says whether the request is open or was turned down", () => {
  const wait = { name: "Rockets", turned_away: false };
  assert.equal(joinWaitStatus(wait).title, "Waiting to join Rockets");
  assert.match(joinWaitStatus(wait).text, /keeps checking/);
  assert.equal(joinWaitStatus({ ...wait, turned_away: true }).title, "Rockets did not let you in");
  assert.deepEqual(joinWaitStatus(null), { title: "", text: "" });
  assert.equal(joinWaitMembers(1), "1 member");
  assert.equal(joinWaitMembers(12), "12 members");
});

test("moderators hear about new requests only once", () => {
  const before = [{ id: "a", name: "Ana" }];
  const after = [{ id: "a", name: "Ana" }, { id: "b", name: "Bo" }];
  assert.deepEqual(newJoinRequests(before, after), [{ id: "b", name: "Bo" }]);
  assert.deepEqual(newJoinRequests(after, after), []);
  assert.equal(joinRequestToast("Rockets", [{ id: "b", name: "Bo" }]), "Bo asked to join Rockets");
  assert.equal(joinRequestToast("Rockets", after), "2 people asked to join Rockets");
  assert.equal(joinRequestToast("Rockets", []), "");
});
