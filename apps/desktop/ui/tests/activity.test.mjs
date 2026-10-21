import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { gameFromActivity, steamStoreUrl, isShowingActivity, splitPlaying, activityArt } = require("../scripts/activity.js");

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

test("people in a game get their own group, longest session first, and leave the rest of the list", () => {
  const members = [
    player("me", "Celeste", 10),
    player("late", "Portal 2", 300),
    player("early", "Hollow Knight", 100),
    player("bot", "Celeste", 1, { bot: true }),
    { id: "idle", name: "idle", presence: "idle", activity: null },
    player("gone", "Celeste", 5, { presence: "offline" }),
  ];
  const { playing, rest } = splitPlaying(members);
  assert.deepEqual(playing.map(m => m.id), ["me", "early", "late"]);
  assert.deepEqual(rest.map(m => m.id), ["bot", "idle", "gone"]);
  assert.equal(playing.length + rest.length, members.length);
});

test("the player's own cover wins, and a looked-up cover fills in when they sent none", () => {
  assert.equal(activityArt({ activity_cover: "co1" }, { cover: "co2" }), "co1");
  assert.equal(activityArt({}, { cover: "co2" }), "co2");
  assert.equal(activityArt({}, null), null);
});

test("the game cards and the Activities picker cards each own their class names, so neither restyles the other", async () => {
  const { readFileSync } = await import("node:fs");
  const css = readFileSync(new URL("../styles/styles.css", import.meta.url), "utf8");
  for (const name of ["activity-card", "activity-chip", "play-card", "now-card", "play-box"]) {
    const heads = css.match(new RegExp(`^\\.${name} \\{`, "gm")) || [];
    assert.equal(heads.length, 1, `.${name} is declared ${heads.length} times`);
  }
});
