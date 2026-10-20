import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  parseSearch, searchIsEmpty, searchArgs, searchToken, searchSuggestions,
  applySearchSuggestion, applySearchFilter, highlightParts, searchExcerpt, searchSummary, unknownFilterText,
} = require("../scripts/search.js");

const members = [
  { id: "u1", name: "Sam" },
  { id: "u2", name: "Sam Lee" },
  { id: "u3", name: "Robin" },
];
const channels = [
  { id: "c1", name: "general" },
  { id: "c2", name: "game-night" },
];
const known = { members, channels };

test("plain words and quoted phrases become terms", () => {
  const q = parseSearch('  raid  "friday night" ', known);
  assert.deepEqual(q.terms, ["raid", "friday night"]);
  assert.equal(q.from, null);
  assert.deepEqual(searchArgs(q), { terms: ["raid", "friday night"], from: null, mentions: null, channelId: null });
});

test("filters pick the longest member or channel name, with or without @ and #", () => {
  const q = parseSearch("from:@Sam Lee mentions:robin in:#game-night snacks", known);
  assert.deepEqual(q.from, { id: "u2", name: "Sam Lee" });
  assert.deepEqual(q.mentions, { id: "u3", name: "Robin" });
  assert.deepEqual(q.channel, { id: "c2", name: "game-night" });
  assert.deepEqual(q.terms, ["snacks"]);
  assert.deepEqual(searchArgs(q), { terms: ["snacks"], from: "u2", mentions: "u3", channelId: "c2" });
  assert.deepEqual(parseSearch("FROM:sam later", known).from, { id: "u1", name: "Sam" });
  assert.deepEqual(parseSearch('from:"sam lee"', known).from, { id: "u2", name: "Sam Lee" });
});

test("a name only counts when it ends at a word boundary", () => {
  const q = parseSearch("from:Samantha", known);
  assert.equal(q.from, null);
  assert.deepEqual(q.unknown, [{ filter: "from", name: "Samantha" }]);
  assert.equal(unknownFilterText(q.unknown), "Nobody called @Samantha in this server");
  assert.equal(unknownFilterText(parseSearch("in:#nowhere", known).unknown), "No channel called #nowhere here");
});

test("a bare @name searches mentions, and an unknown one stays a plain word", () => {
  assert.deepEqual(parseSearch("@Robin", known).mentions, { id: "u3", name: "Robin" });
  const q = parseSearch("@nobody hi", known);
  assert.equal(q.mentions, null);
  assert.deepEqual(q.terms, ["@nobody", "hi"]);
  assert.deepEqual(parseSearch("email@ me", known).terms, ["email@", "me"]);
});

test("an empty query or a lone filter keyword searches nothing", () => {
  assert.ok(searchIsEmpty(parseSearch("", known)));
  assert.ok(searchIsEmpty(parseSearch("   ", known)));
  assert.ok(searchIsEmpty(parseSearch("from:", known)));
  assert.ok(!searchIsEmpty(parseSearch("from:robin", known)));
});

test("the token under the caret drives member and channel suggestions", () => {
  const text = "pizza from:@sa";
  const token = searchToken(text, text.length);
  assert.deepEqual(token, { filter: "from", kind: "member", query: "sa", start: 6, end: text.length });
  assert.deepEqual(searchSuggestions(token, known).map(m => m.name), ["Sam", "Sam Lee"]);
  const inToken = searchToken("in:#ga", 6);
  assert.deepEqual(searchSuggestions(inToken, known).map(c => c.name), ["game-night"]);
  assert.equal(searchToken("pizza", 5), null);
  assert.equal(searchToken("from:robin pizza", 16).query, "robin pizza");
  assert.deepEqual(searchSuggestions(searchToken("from:robin pizza", 16), known), []);
  const bare = searchToken("hi @ro", 6);
  assert.equal(bare.bare, true);
  assert.deepEqual(searchSuggestions(bare, known).map(m => m.id), ["u3"]);
});

test("picking a suggestion writes the full filter and puts the caret after it", () => {
  const text = "pizza from:@sa";
  const token = searchToken(text, text.length);
  assert.deepEqual(applySearchSuggestion(text, token, members[1]), { text: "pizza from:@Sam Lee ", caret: 20 });
  const bare = searchToken("@ro later", 3);
  assert.deepEqual(applySearchSuggestion("@ro later", bare, members[2]), { text: "@Robin later", caret: 7 });
  assert.deepEqual(applySearchFilter("pizza ", "in"), { text: "pizza in:#", caret: 10 });
  assert.deepEqual(applySearchFilter("", "mentions"), { text: "mentions:@", caret: 10 });
  const round = parseSearch(applySearchSuggestion(text, token, members[1]).text, known);
  assert.deepEqual(round.from, { id: "u2", name: "Sam Lee" });
});

test("matched words are marked for highlighting without changing the text", () => {
  const parts = highlightParts("Friday night, FRIDAY!", ["friday", "night"]);
  assert.deepEqual(parts, [
    { text: "Friday", hit: true },
    { text: " ", hit: false },
    { text: "night", hit: true },
    { text: ", ", hit: false },
    { text: "FRIDAY", hit: true },
    { text: "!", hit: false },
  ]);
  assert.equal(parts.map(p => p.text).join(""), "Friday night, FRIDAY!");
  assert.deepEqual(highlightParts("abc", ["ab", "bc"]), [{ text: "abc", hit: true }]);
  assert.deepEqual(highlightParts("abc", []), [{ text: "abc", hit: false }]);
});

test("long messages are cut around the first match", () => {
  assert.equal(searchExcerpt("short one", ["one"]), "short one");
  const long = `${"a ".repeat(200)}needle ${"b ".repeat(200)}`;
  const cut = searchExcerpt(long, ["needle"], 100);
  assert.ok(cut.startsWith("…") && cut.endsWith("…"));
  assert.ok(cut.includes("needle"));
  assert.ok(cut.length <= 102);
  assert.ok(searchExcerpt(long, ["zzz"], 100).startsWith("a a"));
});

test("the summary counts results and says when only the newest are shown", () => {
  assert.equal(searchSummary(0, 0), "No messages found");
  assert.equal(searchSummary(1, 1), "1 result");
  assert.equal(searchSummary(12, 12), "12 results");
  assert.equal(searchSummary(132, 50), "132 results, showing the newest 50");
});
