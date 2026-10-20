import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { gameFromActivity, steamStoreUrl, isShowingActivity, playingNow, activityArt, PLAYING_NOW_LIMIT } = require("../scripts/activity.js");

const player = (id, game, since, extra = {}) => ({ id, name: id, presence: "online", activity: `Playing ${game}`, activity_since: since, ...extra });

test("only a Playing activity names a game", () => {
  assert.equal(gameFromActivity("Playing Hollow Knight"), "Hollow Knight");
  assert.equal(gameFromActivity("Using Visual Studio Code"), null);
  assert.equal(gameFromActivity("Playing   "), null);
  assert.equal(gameFromActivity(null), null);
});

test("the Steam button links only to a real store page", () => {
  assert.equal(steamStoreUrl(367520), "https://store.steampowered.com/app/367520/");
  assert.equal(steamStoreUrl("620"), "https://store.steampowered.com/app/620/");
  for (const bad of [0, -1, 1.5, 2 ** 32, "12/../evil", null, undefined]) assert.equal(steamStoreUrl(bad), null, String(bad));
});

test("offline and invisible people never show a game", () => {
  assert.equal(isShowingActivity(player("a", "Celeste", 1)), true);
  assert.equal(isShowingActivity(player("a", "Celeste", 1, { presence: "offline" })), false);
  assert.equal(isShowingActivity(player("a", "Celeste", 1, { presence: "invisible" })), false);
  assert.equal(isShowingActivity({ id: "a", presence: "online", activity: "Using Notepad" }), false);
});

test("playing now lists other people in a game, longest session first, without you or bots", () => {
  const members = [
    player("me", "Celeste", 10),
    player("late", "Portal 2", 300),
    player("early", "Hollow Knight", 100),
    player("bot", "Celeste", 1, { bot: true }),
    { id: "idle", name: "idle", presence: "idle", activity: null },
  ];
  assert.deepEqual(playingNow(members, "me").map(m => m.id), ["early", "late"]);
});

test("playing now is capped so the people list keeps its shape", () => {
  const crowd = Array.from({ length: 9 }, (_, i) => player(`p${i}`, "Celeste", 100 + i));
  assert.equal(playingNow(crowd, null).length, PLAYING_NOW_LIMIT);
});

test("the player's own cover wins, and a looked-up cover fills in when they sent none", () => {
  assert.equal(activityArt({ activity_cover: "co1" }, { cover: "co2" }), "co1");
  assert.equal(activityArt({}, { cover: "co2" }), "co2");
  assert.equal(activityArt({}, null), null);
});
