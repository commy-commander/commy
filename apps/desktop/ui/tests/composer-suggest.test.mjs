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

test("bot command names make the slash token a command even with arguments, unless a mention is being typed", () => {
  const names = ["roll", "remind", "roll-d20"];
  assert.deepEqual(suggestTokenAt("/roll 20", 8, names), { kind: "command", start: 0, query: "roll", args: "20" });
  assert.deepEqual(suggestTokenAt("/roll-d20 ", 10, names), { kind: "command", start: 0, query: "roll-d20", args: "" });
  assert.deepEqual(suggestTokenAt("/remind @no", 11, names), { kind: "mention", start: 8, query: "no" });
  assert.deepEqual(suggestTokenAt("/remind @nova 10", 16, names), { kind: "command", start: 0, query: "remind", args: "@nova 10" });
  assert.equal(suggestTokenAt("/dance now", 10, names), null);
  assert.deepEqual(suggestTokenAt("/ro", 3, names), { kind: "command", start: 0, query: "ro", args: null });
});

test("bot commands join the command list with their entry and typed arguments", () => {
  const catalog = [
    { bot: "b1", bot_name: "Tidy", name: "remind", description: "Remind someone", options: [{ name: "who", required: true, kind: "user" }] },
    { bot: "b1", bot_name: "Tidy", name: "roll", description: "Roll dice", options: [] },
  ];
  const partial = commandSuggestions({ kind: "command", start: 0, query: "r", args: null }, {}, catalog);
  assert.deepEqual(partial.map(i => [i.name, i.bot, i.args, i.ready]), [["remind", true, null, false], ["roll", true, null, false]]);
  assert.equal(partial[0].entry.description, "Remind someone");
  const typed = commandSuggestions({ kind: "command", start: 0, query: "roll", args: "20" }, {}, catalog);
  assert.deepEqual(typed.map(i => [i.name, i.args, i.ready]), [["roll", "20", true]]);
  const both = commandSuggestions({ kind: "command", start: 0, query: "", args: null }, { gifs: true }, catalog);
  assert.deepEqual(both.map(i => `${i.name}${i.bot ? "*" : ""}`), ["gif", "tts", "remind*", "roll*"]);
  assert.equal(suggestionInsert(typed[0]), "/roll ");
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

test("emoticons are suggested by name and insert their text", () => {
  const faces = { name: "Commy", emojis: [], emoticons: [{ name: "shrug", text: "¯\\_(ツ)_/¯" }, { name: "table_flip", text: "(╯°□°)╯︵ ┻━┻" }] };
  const list = emojiSuggestions("shr", [here, faces], STANDARD);
  assert.deepEqual(list.map(e => [e.kind, e.name]), [["emoticon", "shrug"]]);
  assert.equal(suggestionInsert(list[0]), "¯\\_(ツ)_/¯ ");
  assert.deepEqual(emojiSuggestions("flip", [faces], STANDARD).map(e => e.text), ["(╯°□°)╯︵ ┻━┻"]);
  const same = { name: "Art Club", emojis: [{ name: "shrug", image: "z@p" }], emoticons: [] };
  assert.deepEqual(emojiSuggestions("shrug", [same, faces], STANDARD).map(e => e.kind), ["custom", "emoticon"], "an emoji and an emoticon may share a name");
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

test("/gif is always offered and says when GIF search is not set up", () => {
  assert.deepEqual(commandSuggestions(suggestTokenAt("/g"), { gifs: true }), [{ kind: "command", name: "gif", args: "", ready: false, available: true }]);
  assert.deepEqual(commandSuggestions(suggestTokenAt("/gif"), { gifs: true }), [{ kind: "command", name: "gif", args: "", ready: true, available: true }]);
  assert.deepEqual(commandSuggestions(suggestTokenAt("/gif  space cats "), { gifs: true }), [{ kind: "command", name: "gif", args: "space cats", ready: true, available: true }]);
  assert.deepEqual(commandSuggestions(suggestTokenAt("/g"), { gifs: false }), [{ kind: "command", name: "gif", args: "", ready: false, available: false }]);
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
