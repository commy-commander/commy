import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { streamOpenAction, callTiles, callGridSize, callTilePages, callGridLayout, streamsToStop } = require("../scripts/stream-view.js");

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



const room = {
  userIds: ["me", "a", "b", "c"],
  names: ["commy", "ash", "bo", "cy"],
  meId: "me",
  channelId: "voice",
};

test("the grid holds everyone in your channel who streams or has a camera on, streams first", () => {
  const tiles = callTiles({
    ...room,
    cameras: ["c", "me", "a"],
    shares: [{ user: "me", name: "commy", channel: "voice" }, { user: "b", name: "bo", channel: "voice" }],
  });
  assert.deepEqual(tiles.map(t => t.key), ["screen:b", "screen:me", "camera:me", "camera:c", "camera:a"]);
  assert.equal(tiles.find(t => t.key === "screen:me").me, true);
  assert.equal(tiles.find(t => t.key === "screen:b").name, "bo");
});

test("streams and cameras from people outside your channel stay out of the grid", () => {
  const tiles = callTiles({
    ...room,
    cameras: ["z"],
    shares: [{ user: "z", name: "zed", channel: "voice" }, { user: "a", name: "ash", channel: "other" }],
  });
  assert.deepEqual(tiles, []);
});

test("the grid picks the column count that gives the biggest tiles", () => {
  assert.deepEqual(callGridSize(1, 800, 450, 8), { cols: 1, tileWidth: 800 });
  assert.equal(callGridSize(2, 900, 300, 8).cols, 2);
  assert.equal(callGridSize(4, 900, 520, 8).cols, 2);
  assert.equal(callGridSize(6, 1100, 420, 8).cols, 3);
  assert.equal(callGridSize(3, 300, 900, 8).cols, 1);
  assert.deepEqual(callGridSize(4, 0, 400), { cols: 1, tileWidth: 0 });
});

test("the phone splits tiles into pages of four to swipe through", () => {
  const tiles = [1, 2, 3, 4, 5, 6].map(n => ({ key: String(n) }));
  assert.deepEqual(callTilePages(tiles).map(p => p.map(t => t.key)), [["1", "2", "3", "4"], ["5", "6"]]);
  assert.deepEqual(callTilePages([], 4), []);
  assert.deepEqual(callTilePages(tiles, 0).length, 6);
});

test("a stream on the big screen turns the grid into a row of tiles under it", () => {
  assert.equal(callGridLayout({ tiles: [], focusedStream: null }), "none");
  assert.equal(callGridLayout({ tiles: [{}], focusedStream: null }), "grid");
  assert.equal(callGridLayout({ tiles: [{}], focusedStream: "b" }), "strip");
});

test("streams leave only once they have neither a tile nor the big screen", () => {
  assert.deepEqual(streamsToStop(new Set(["a", "b", "c"]), { tileUsers: ["a"], focused: "c" }), ["b"]);
  assert.deepEqual(streamsToStop(new Set(["a"]), { tileUsers: [], focused: null }), ["a"]);
  assert.deepEqual(streamsToStop(new Set(), { tileUsers: ["a"], focused: "a" }), []);
});
