import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { SUGGEST_LIMIT, suggestTokenAt, emojiSuggestions, commandSuggestions, suggestionInsert } = require("../scripts/composer-suggest.js");
const { playsInline, MAX_INLINE_VIDEO_BYTES } = require("../scripts/embeds.js");

const STANDARD = [["\u{1F44D}", "thumbs up"], ["\u{1F525}", "fire lit"], ["\u{1F980}", "crab"], ["\u{1F389}", "party popper tada"]];
const here = { name: "Rust Hangout", emojis: [{ name: "ferris", image: "a@p" }, { name: "party_ferris", image: "b@p" }] };
const elsewhere = { name: "Art Club", emojis: [{ name: "ferris", image: "c@p" }, { name: "fern", image: "d@p" }] };

test("a colon and two letters at a word start asks for emoji", () => {
  assert.deepEqual(suggestTokenAt(":fe", 3), { kind: "emoji", start: 0, query: "fe" });
  assert.deepEqual(suggestTokenAt("nice :Fer", 9), { kind: "emoji", start: 5, query: "fer" });
  assert.equal(suggestTokenAt(":f", 2), null);
  assert.equal(suggestTokenAt("at 12:30", 8), null);
  assert.equal(suggestTokenAt("https://x", 9), null);
  assert.equal(suggestTokenAt(":ferris: ", 9), null);
  assert.deepEqual(suggestTokenAt(":fer and more", 4), { kind: "emoji", start: 0, query: "fer" });
});

test("an @ still asks for members", () => {
  assert.deepEqual(suggestTokenAt("hey @no", 7), { kind: "mention", start: 4, query: "no" });
  assert.deepEqual(suggestTokenAt("@", 1), { kind: "mention", start: 0, query: "" });
  assert.equal(suggestTokenAt("mail@host", 9), null);
});

test("a slash at the start asks for commands, with what follows as the search", () => {
  assert.deepEqual(suggestTokenAt("/", 1), { kind: "command", start: 0, query: "", args: null });
  assert.deepEqual(suggestTokenAt("/gi", 3), { kind: "command", start: 0, query: "gi", args: null });
  assert.deepEqual(suggestTokenAt("/GIF cats", 9), { kind: "command", start: 0, query: "gif", args: "cats" });
  assert.deepEqual(suggestTokenAt("/gif ", 5), { kind: "command", start: 0, query: "gif", args: "" });
  assert.equal(suggestTokenAt("a /gif", 6), null);
  assert.equal(suggestTokenAt("/usr/bin", 8), null);
  assert.deepEqual(suggestTokenAt("/me hi @no", 10), { kind: "mention", start: 7, query: "no" });
});

test("custom emoji come first, this server before others, one per name", () => {
  const list = emojiSuggestions("fer", [here, elsewhere], STANDARD);
  assert.deepEqual(list.map(e => [e.kind, e.name, e.server]), [
    ["custom", "ferris", "Rust Hangout"],
    ["custom", "fern", "Art Club"],
    ["custom", "party_ferris", "Rust Hangout"],
  ]);
  assert.equal(list[0].image, "a@p");
});

test("standard emoji match any word of their name", () => {
  assert.deepEqual(emojiSuggestions("ta", [], STANDARD).map(e => e.emoji), ["\u{1F389}"]);
  assert.deepEqual(emojiSuggestions("thumbs_u", [], STANDARD).map(e => e.emoji), ["\u{1F44D}"]);
  assert.deepEqual(emojiSuggestions("cr", [here], STANDARD).map(e => e.kind), ["standard"]);
  assert.deepEqual(emojiSuggestions("f", [here], STANDARD), []);
});

test("emoji suggestions stop at the limit", () => {
  const many = { name: "Big", emojis: Array.from({ length: 20 }, (_, i) => ({ name: `cat_${i}`, image: `${i}@p` })) };
  assert.equal(emojiSuggestions("cat", [many], STANDARD).length, SUGGEST_LIMIT);
});

test("/gif is offered only where GIFs work", () => {
  assert.deepEqual(commandSuggestions(suggestTokenAt("/g"), { gifs: true }), [{ kind: "command", name: "gif", args: "", ready: false }]);
  assert.deepEqual(commandSuggestions(suggestTokenAt("/gif"), { gifs: true }), [{ kind: "command", name: "gif", args: "", ready: true }]);
  assert.deepEqual(commandSuggestions(suggestTokenAt("/gif  space cats "), { gifs: true }), [{ kind: "command", name: "gif", args: "space cats", ready: true }]);
  assert.deepEqual(commandSuggestions(suggestTokenAt("/g"), { gifs: false }), []);
  assert.deepEqual(commandSuggestions(suggestTokenAt("/x"), { gifs: true }), []);
});

test("picking a suggestion inserts the right text", () => {
  assert.equal(suggestionInsert({ kind: "custom", name: "ferris" }), ":ferris: ");
  assert.equal(suggestionInsert({ kind: "standard", emoji: "\u{1F525}" }), "\u{1F525} ");
  assert.equal(suggestionInsert({ kind: "command", name: "gif" }), "/gif ");
  assert.equal(suggestionInsert({ kind: "mention", name: "nova" }), "@nova ");
});

test("common video files play inline up to the size cap", () => {
  assert.equal(playsInline({ mime: "video/mp4", size_bytes: 1000 }), true);
  assert.equal(playsInline({ mime: "video/QuickTime", size_bytes: 1000 }), true);
  assert.equal(playsInline({ mime: "video/x-matroska", size_bytes: 1000 }), false);
  assert.equal(playsInline({ mime: "image/png", size_bytes: 1000 }), false);
  assert.equal(playsInline({ mime: "video/webm", size_bytes: MAX_INLINE_VIDEO_BYTES + 1 }), false);
  assert.equal(playsInline(null), false);
});
