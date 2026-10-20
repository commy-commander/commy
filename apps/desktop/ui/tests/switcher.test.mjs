import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  SWITCHER_RECENT_CAP, parseSwitcherQuery, switcherScore, rankSwitcher, rememberRecent, readRecent, buildSwitcherItems,
} = require("../scripts/switcher.js");

const items = [
  { key: "channel:c1", kind: "channel", name: "#general", subtitle: "Rust Hangout" },
  { key: "channel:c2", kind: "channel", name: "#game-night", subtitle: "Rust Hangout" },
  { key: "channel:c3", kind: "channel", name: "#gen-z-memes", subtitle: "Pixel Club" },
  { key: "dm:m1", kind: "person", name: "ferris", subtitle: "Online" },
  { key: "dm:m2", kind: "person", name: "nova", subtitle: "Idle" },
  { key: "server:s1", kind: "server", name: "Rust Hangout", subtitle: "Server" },
  { key: "action:settings", kind: "action", name: "Settings", subtitle: "" },
];

test("prefixes pick a kind and are stripped from the text", () => {
  assert.deepEqual(parseSwitcherQuery("  #Gen "), { kind: "channel", text: "gen" });
  assert.deepEqual(parseSwitcherQuery("@fer"), { kind: "person", text: "fer" });
  assert.deepEqual(parseSwitcherQuery("*rust"), { kind: "server", text: "rust" });
  assert.deepEqual(parseSwitcherQuery("plain"), { kind: null, text: "plain" });
  assert.deepEqual(parseSwitcherQuery(""), { kind: null, text: "" });
});

test("an exact prefix beats a substring, which beats a subsequence", () => {
  const prefix = switcherScore("gen", { name: "#general" });
  const substring = switcherScore("gen", { name: "#urgent" });
  const subsequence = switcherScore("gen", { name: "#game-night" });
  assert.ok(prefix > substring, `${prefix} > ${substring}`);
  assert.ok(substring > subsequence, `${substring} > ${subsequence}`);
  assert.equal(switcherScore("xyz", { name: "#general", subtitle: "Rust Hangout" }), -1);
});

test("the leading # of a channel name does not block a prefix match", () => {
  assert.ok(switcherScore("gen", { name: "#general" }) > switcherScore("gen", { name: "#gen-z-memes" }));
  const ranked = rankSwitcher(items, "gen").map(i => i.key);
  assert.deepEqual(ranked.slice(0, 2), ["channel:c1", "channel:c3"]);
  assert.ok(ranked.includes("channel:c2"));
});

test("matching is case-insensitive and tighter subsequences rank higher", () => {
  assert.equal(switcherScore("GENERAL", { name: "#general" }), switcherScore("general", { name: "#general" }));
  assert.ok(switcherScore("gn", { name: "#gnats" }) > switcherScore("gn", { name: "#game-night" }));
});

test("a server name matches a channel through its subtitle, but below any name match", () => {
  const viaSub = switcherScore("rust", { name: "#general", subtitle: "Rust Hangout" });
  assert.ok(viaSub > 0);
  assert.ok(viaSub < switcherScore("rust", { name: "#rusty-nails", subtitle: "" }));
  assert.ok(viaSub < switcherScore("rust", { name: "#crusty", subtitle: "" }));
  const ranked = rankSwitcher(items, "rust").map(i => i.key);
  assert.equal(ranked[0], "server:s1");
  assert.ok(ranked.includes("channel:c1"));
});

test("with an empty query recent items come first in recency order, then the rest in list order", () => {
  const ranked = rankSwitcher(items, "", ["dm:m2", "channel:c2"]).map(i => i.key);
  assert.deepEqual(ranked.slice(0, 2), ["dm:m2", "channel:c2"]);
  assert.deepEqual(ranked.slice(2), ["channel:c1", "channel:c3", "dm:m1", "server:s1", "action:settings"]);
});

test("recency only breaks ties between equal scores", () => {
  const ranked = rankSwitcher(items, "n", ["dm:m2"]).map(i => i.key);
  assert.equal(ranked[0], "dm:m2");
  const tied = rankSwitcher([
    { key: "a", kind: "channel", name: "#alpha" },
    { key: "b", kind: "channel", name: "#alpho" },
  ], "alph", ["b"]).map(i => i.key);
  assert.deepEqual(tied, ["b", "a"]);
});

test("# limits to channels, @ to people and * to servers", () => {
  assert.ok(rankSwitcher(items, "#").every(i => i.kind === "channel"));
  assert.equal(rankSwitcher(items, "#").length, 3);
  assert.deepEqual(rankSwitcher(items, "@nov").map(i => i.key), ["dm:m2"]);
  assert.deepEqual(rankSwitcher(items, "@n").map(i => i.key), ["dm:m2", "dm:m1"]);
  assert.deepEqual(rankSwitcher(items, "*").map(i => i.key), ["server:s1"]);
  assert.deepEqual(rankSwitcher(items, "@zzz"), []);
});

test("recents move to the front, drop duplicates and stay capped", () => {
  let recent = [];
  for (let i = 0; i < 30; i++) recent = rememberRecent(recent, `channel:${i}`);
  assert.equal(recent.length, SWITCHER_RECENT_CAP);
  assert.equal(recent[0], "channel:29");
  recent = rememberRecent(recent, "channel:12");
  assert.equal(recent[0], "channel:12");
  assert.equal(recent.filter(k => k === "channel:12").length, 1);
  assert.equal(recent.length, SWITCHER_RECENT_CAP);
});

test("stored recents are read defensively", () => {
  assert.deepEqual(readRecent(null), []);
  assert.deepEqual(readRecent("not json"), []);
  assert.deepEqual(readRecent('{"a":1}'), []);
  assert.deepEqual(readRecent('["dm:m1", 3, "channel:c1"]'), ["dm:m1", "channel:c1"]);
  assert.equal(readRecent(JSON.stringify(Array.from({ length: 40 }, (_, i) => `k${i}`))).length, SWITCHER_RECENT_CAP);
});

test("items cover visible channels of every kind, conversations, friends, servers and actions", () => {
  const servers = [
    { id: "s1", name: "Rust Hangout", group: false },
    { id: "g1", name: "ferris, nova", group: true, member_count: 2 },
  ];
  const views = {
    s1: { channels: [
      { id: "c1", name: "general", kind: "text" },
      { id: "c2", name: "Lounge", kind: "voice" },
      { id: "c3", name: "news", kind: "announcement" },
      { id: "c4", name: "Games", kind: "category" },
      { id: "c5", name: "mod-lounge", kind: "text", can_view: false },
    ] },
    g1: { channels: [{ id: "gc", name: "chat", kind: "text" }] },
  };
  const threads = [
    { user: "m1", name: "ferris", kind: "inbox" },
    { user: "g1", name: "ferris, nova", kind: "group" },
  ];
  const friends = [{ id: "m1", name: "ferris" }, { id: "m2", name: "nova" }];
  const built = buildSwitcherItems({
    servers, views, threads, friends, presence: id => (id === "m2" ? "Idle" : ""),
    actions: [{ id: "settings", name: "Settings" }],
  });
  assert.deepEqual(built.map(i => i.key), [
    "server:s1", "channel:c1", "channel:c2", "channel:c3", "channel:gc", "dm:m1", "group:g1", "dm:m2", "action:settings",
  ]);
  const byKey = Object.fromEntries(built.map(i => [i.key, i]));
  assert.equal(byKey["channel:c1"].name, "#general");
  assert.equal(byKey["channel:c1"].subtitle, "Rust Hangout");
  assert.equal(byKey["channel:c2"].name, "Lounge");
  assert.equal(byKey["channel:c2"].channelKind, "voice");
  assert.equal(byKey["channel:gc"].subtitle, "ferris, nova");
  assert.equal(byKey["dm:m1"].subtitle, "Direct message");
  assert.equal(byKey["dm:m2"].subtitle, "Idle");
  assert.equal(byKey["group:g1"].group, true);
  assert.equal(byKey["group:g1"].kind, "person");
  assert.equal(byKey["server:s1"].kind, "server");
  assert.equal(byKey["action:settings"].kind, "action");
});

test("channel names go through the plain-name hook so emoji codes do not leak", () => {
  const built = buildSwitcherItems({
    servers: [{ id: "s1", name: "Rust Hangout" }],
    views: { s1: { channels: [{ id: "c1", name: ":ferris:-rust-help", kind: "text" }] } },
    plainName: c => c.name.replace(/:[a-z_]+:-?/g, ""),
  });
  assert.equal(built.find(i => i.kind === "channel").name, "#rust-help");
});
