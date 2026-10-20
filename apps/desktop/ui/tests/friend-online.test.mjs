import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  FRIEND_ONLINE_WINDOW_SECS, isVisiblyOnline, cameOnline, mergedPresence, wantsFriendOnline, withFriendOnlinePick,
  createFriendOnlineTracker, sweepFriendPresence, friendOnlineNotice,
} = require("../scripts/friend-online.js");

const ferris = { id: "m1", name: "ferris" };
const nova = { id: "m2", name: "nova" };
const friends = [ferris, nova];
const everyone = { friendOnline: true };

function sweep(tracker, presences, { prefs = everyone, myPresence = "online", nowSecs = 1000 } = {}) {
  return sweepFriendPresence(tracker, { friends, presenceOf: id => presences[id], prefs, myPresence, nowSecs });
}

test("idle and Do Not Disturb count as online, offline and unknown do not", () => {
  assert.equal(isVisiblyOnline("online"), true);
  assert.equal(isVisiblyOnline("idle"), true);
  assert.equal(isVisiblyOnline("do_not_disturb"), true);
  assert.equal(isVisiblyOnline("offline"), false);
  assert.equal(isVisiblyOnline("invisible"), false);
  assert.equal(isVisiblyOnline(undefined), false);
  assert.equal(cameOnline("offline", "online"), true);
  assert.equal(cameOnline("offline", "idle"), true);
  assert.equal(cameOnline("idle", "online"), false);
  assert.equal(cameOnline(undefined, "online"), false);
  assert.equal(cameOnline("online", "offline"), false);
});

test("presence across server views: any view that sees them online wins", () => {
  const views = [[{ id: "m1", presence: "offline" }], [{ id: "m1", presence: "idle" }, { id: "m2", presence: "offline" }]];
  assert.equal(mergedPresence(views, "m1"), "idle");
  assert.equal(mergedPresence(views, "m2"), "offline");
  assert.equal(mergedPresence(views, "m9"), null);
  assert.equal(mergedPresence([], "m1"), null);
});

test("global off means only the friends turned on, global on means everyone except those turned off", () => {
  assert.equal(wantsFriendOnline({}, "m1"), false);
  assert.equal(wantsFriendOnline({ friendOnline: false, friendOnlinePicks: { m1: true } }, "m1"), true);
  assert.equal(wantsFriendOnline({ friendOnline: false, friendOnlinePicks: { m1: true } }, "m2"), false);
  assert.equal(wantsFriendOnline({ friendOnline: true }, "m2"), true);
  assert.equal(wantsFriendOnline({ friendOnline: true, friendOnlinePicks: { m2: false } }, "m2"), false);
  assert.equal(wantsFriendOnline({ friendOnline: true, friendOnlinePicks: { m2: false } }, "m1"), true);
});

test("a per-friend pick is kept only while it differs from the global switch", () => {
  const level = { level: "all", friendOnline: false };
  const on = withFriendOnlinePick(level, "m1", true);
  assert.deepEqual(on, { level: "all", friendOnline: false, friendOnlinePicks: { m1: true } });
  assert.equal(wantsFriendOnline(on, "m1"), true);
  const back = withFriendOnlinePick(on, "m1", false);
  assert.deepEqual(back, { level: "all", friendOnline: false });
  const cleared = withFriendOnlinePick(on, "m1", null);
  assert.equal("friendOnlinePicks" in cleared, false);
  const quietOne = withFriendOnlinePick({ friendOnline: true }, "m2", false);
  assert.deepEqual(quietOne.friendOnlinePicks, { m2: false });
  assert.equal(wantsFriendOnline(quietOne, "m2"), false);
  assert.equal(wantsFriendOnline(quietOne, "m1"), true);
});

test("the first sweep after start is silent even when friends are already online", () => {
  const tracker = createFriendOnlineTracker();
  assert.deepEqual(sweep(tracker, { m1: "online", m2: "online" }), []);
  assert.deepEqual(sweep(tracker, { m1: "online", m2: "online" }), []);
});

test("a friend going from offline to online is announced once", () => {
  const tracker = createFriendOnlineTracker();
  sweep(tracker, { m1: "offline", m2: "online" });
  assert.deepEqual(sweep(tracker, { m1: "online", m2: "online" }), [ferris]);
  assert.deepEqual(sweep(tracker, { m1: "online", m2: "online" }), []);
  assert.deepEqual(sweep(tracker, { m1: "online", m2: "idle" }), []);
});

test("a friend added to the list later is not announced until they actually come online", () => {
  const tracker = createFriendOnlineTracker();
  const only = list => presences => sweepFriendPresence(tracker, { friends: list, presenceOf: id => presences[id], prefs: everyone, myPresence: "online", nowSecs: 1000 });
  only([ferris])({ m1: "offline" });
  assert.deepEqual(only([ferris, nova])({ m1: "idle", m2: "online" }), [ferris]);
  assert.deepEqual(only([ferris, nova])({ m1: "idle", m2: "offline" }), []);
  assert.deepEqual(only([ferris, nova])({ m1: "idle", m2: "online" }), [nova]);
});

test("a flapping connection gets one toast per friend per ten minutes", () => {
  assert.equal(FRIEND_ONLINE_WINDOW_SECS, 600);
  const tracker = createFriendOnlineTracker();
  sweep(tracker, { m1: "offline" }, { nowSecs: 0 });
  assert.deepEqual(sweep(tracker, { m1: "online" }, { nowSecs: 10 }), [ferris]);
  sweep(tracker, { m1: "offline" }, { nowSecs: 20 });
  assert.deepEqual(sweep(tracker, { m1: "online" }, { nowSecs: 30 }), []);
  sweep(tracker, { m1: "offline" }, { nowSecs: 400 });
  assert.deepEqual(sweep(tracker, { m1: "online" }, { nowSecs: 609 }), []);
  sweep(tracker, { m1: "offline" }, { nowSecs: 615 });
  assert.deepEqual(sweep(tracker, { m1: "online" }, { nowSecs: 620 }), [ferris]);
});

test("Do Not Disturb and appearing offline keep it quiet, and friends who came online meanwhile are not announced afterwards", () => {
  const tracker = createFriendOnlineTracker();
  sweep(tracker, { m1: "offline", m2: "offline" });
  assert.deepEqual(sweep(tracker, { m1: "online", m2: "offline" }, { myPresence: "do_not_disturb" }), []);
  assert.deepEqual(sweep(tracker, { m1: "online", m2: "online" }, { myPresence: "invisible" }), []);
  assert.deepEqual(sweep(tracker, { m1: "online", m2: "online" }, { myPresence: "online" }), []);
  sweep(tracker, { m1: "offline", m2: "online" }, { nowSecs: 2000 });
  assert.deepEqual(sweep(tracker, { m1: "online", m2: "online" }, { myPresence: "idle", nowSecs: 2001 }), [ferris]);
});

test("the switch and the per-friend picks decide who is announced", () => {
  const tracker = createFriendOnlineTracker();
  sweep(tracker, { m1: "offline", m2: "offline" }, { prefs: {} });
  assert.deepEqual(sweep(tracker, { m1: "online", m2: "online" }, { prefs: {} }), []);
  sweep(tracker, { m1: "offline", m2: "offline" }, { prefs: {} });
  assert.deepEqual(sweep(tracker, { m1: "online", m2: "online" }, { prefs: { friendOnlinePicks: { m2: true } } }), [nova]);
  sweep(tracker, { m1: "offline", m2: "offline" }, { prefs: {} });
  assert.deepEqual(sweep(tracker, { m1: "online", m2: "online" }, { prefs: { friendOnline: true, friendOnlinePicks: { m2: false } }, nowSecs: 5000 }), [ferris]);
});

test("a presence the views do not know counts as offline", () => {
  const tracker = createFriendOnlineTracker();
  sweep(tracker, {});
  assert.deepEqual(sweep(tracker, { m1: "online" }), [ferris]);
});

test("the notice names the friend and opens their DM", () => {
  assert.deepEqual(friendOnlineNotice(ferris), { title: "ferris is now online", body: "Click to send a message", target: "dm:m1" });
});
