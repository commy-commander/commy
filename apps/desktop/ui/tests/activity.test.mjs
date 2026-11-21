import "./i18n-global.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { tidyGameName, gameFromActivity, shownActivity, steamStoreUrl, isShowingActivity, splitPlaying, activityArt, sharesOnlyGames } = require("../scripts/activity.js");

const player = (id, game, since, extra = {}) => ({ id, name: id, presence: "online", activity: `Playing ${game}`, activity_since: since, ...extra });

test("only a Playing activity names a game", () => {
  assert.equal(gameFromActivity("Playing Hollow Knight"), "Hollow Knight");
  assert.equal(gameFromActivity("Using Visual Studio Code"), null);
  assert.equal(gameFromActivity("Playing   "), null);
  assert.equal(gameFromActivity(null), null);
});

test("engine build names are cleaned up before they are shown", () => {
  assert.equal(gameFromActivity("Playing WardogsLauncher Shipping"), "Wardogs");
  assert.equal(gameFromActivity("Playing FortniteClient-Win64-Shipping"), "Fortnite");
  assert.equal(tidyGameName("League of Legends Client"), "League of Legends");
  assert.equal(tidyGameName("Counter-Strike 2"), "Counter-Strike 2");
  assert.equal(tidyGameName("Launcher"), "Launcher");
  assert.equal(tidyGameName("RocketLeague"), "RocketLeague");
  assert.equal(tidyGameName("Cyberpunk2077"), "Cyberpunk2077");
  assert.equal(tidyGameName("Hollow Knight"), "Hollow Knight");
  assert.equal(shownActivity("Playing WardogsLauncher Shipping"), "Playing Wardogs");
  assert.equal(shownActivity("Using Blender"), "Using Blender");
  assert.equal(shownActivity(null), null);
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

test("activity sharing shows only games unless the person turns that off", () => {
  assert.equal(sharesOnlyGames({}), true);
  assert.equal(sharesOnlyGames(null), true);
  assert.equal(sharesOnlyGames({ activity_games_only: true }), true);
  assert.equal(sharesOnlyGames({ activity_games_only: false }), false);
});

test("the games only switch sits under activity sharing and follows it", async () => {
  const { readFileSync } = await import("node:fs");
  const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
  const app = readFileSync(new URL("../scripts/app.js", import.meta.url), "utf8");
  assert.ok(html.indexOf('id="activity-games-only-toggle"') > html.indexOf('id="share-activity-toggle"'));
  assert.match(app, /\$\("activity-games-only-toggle"\)\.disabled = !sharing;/);
  assert.match(app, /invoke\("set_activity_games_only", \{ enabled: e\.target\.checked \}\)/);
});

test("a long name in the Playing list gives way so the time played stays in view", async () => {
  const { readFileSync } = await import("node:fs");
  const css = readFileSync(new URL("../styles/studio.css", import.meta.url), "utf8");
  const rule = css.match(/\.now-card \.acc-text \{([^}]*)\}/);
  assert.ok(rule, "the Playing card text grid is styled");
  assert.match(rule[1], /grid-template-columns: minmax\(0, max-content\) max-content;/);
});
