import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { streamOpenAction } = require("../scripts/stream-view.js");

test("your own stream opens your preview instead of dialing your own device", () => {
  assert.equal(streamOpenAction({ userId: "me", meId: "me", sharing: true, watchingUser: null }), "preview");
  assert.equal(streamOpenAction({ userId: "me", meId: "me", sharing: true, watchingUser: "b" }), "preview");
});

test("your stream from another of your devices is watched like anyone else's", () => {
  assert.equal(streamOpenAction({ userId: "me", meId: "me", sharing: false, watchingUser: null }), "watch");
});

test("the stream you already watch is kept without a second dial", () => {
  assert.equal(streamOpenAction({ userId: "b", meId: "me", sharing: true, watchingUser: "b" }), "already");
});

test("another person's stream is watched", () => {
  assert.equal(streamOpenAction({ userId: "b", meId: "me", sharing: false, watchingUser: null }), "watch");
  assert.equal(streamOpenAction({ userId: "c", meId: "me", sharing: true, watchingUser: "b" }), "watch");
});
