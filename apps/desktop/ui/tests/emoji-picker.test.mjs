import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { EMOJI_TONES, EMOJI_GROUPS, EMOJI_DATA } = require("../scripts/emoji.js");
const {
  SKIN_FIRST, SKIN_EVERY_PERSON, FREQUENT_KEEP,
  emojiRow, withSkinTone, searchEmoji, emojiByGroup, emojiCodesLabel,
  readFrequentEmoji, recordEmojiUse, frequentEmoji, emojiVersionProbes, highestSupportedVersion,
  splitLeadingEmoji, joinLeadingEmoji,
} = require("../scripts/emoji-picker.js");

const row = emoji => EMOJI_DATA.find(r => r[0] === emoji);
const first = (query, opts) => searchEmoji(EMOJI_DATA, query, opts)[0].entry.emoji;

test("the bundled set is the full emoji list, every row in a known category with a name to search by", () => {
  assert.ok(EMOJI_DATA.length > 1800, `only ${EMOJI_DATA.length} emoji`);
  assert.equal(EMOJI_GROUPS.length, 9);
  assert.equal(new Set(EMOJI_DATA.map(r => r[0])).size, EMOJI_DATA.length, "no emoji is listed twice");
  for (const r of EMOJI_DATA) {
    const entry = emojiRow(r);
    assert.ok(EMOJI_GROUPS[entry.group], `${entry.label} has no category`);
    assert.ok(entry.label && entry.codes.length, `${entry.emoji} has no name or shortcode`);
    assert.ok(Number.isFinite(entry.version));
    assert.ok([0, SKIN_FIRST, SKIN_EVERY_PERSON].includes(entry.skin));
  }
  assert.ok(emojiByGroup(EMOJI_DATA, EMOJI_GROUPS).every(section => section.items.length > 50));
});

test("skin tones land where each emoji expects them", () => {
  const tone = n => EMOJI_TONES[n - 1];
  const thumbs = emojiRow(row("\u{1F44D}"));
  assert.equal(withSkinTone(thumbs.emoji, thumbs.skin, 3, EMOJI_TONES), `\u{1F44D}${tone(3)}`);
  assert.equal(withSkinTone(thumbs.emoji, thumbs.skin, 0, EMOJI_TONES), thumbs.emoji);
  const raised = emojiRow(row("\u{261D}\u{FE0F}"));
  assert.equal(withSkinTone(raised.emoji, raised.skin, 1, EMOJI_TONES), `\u{261D}${tone(1)}`, "the presentation selector gives way to the tone");
  const holding = emojiRow(row("\u{1F9D1}\u{200D}\u{1F91D}\u{200D}\u{1F9D1}"));
  assert.equal(holding.skin, SKIN_EVERY_PERSON);
  assert.equal(withSkinTone(holding.emoji, holding.skin, 5, EMOJI_TONES), `\u{1F9D1}${tone(5)}\u{200D}\u{1F91D}\u{200D}\u{1F9D1}${tone(5)}`);
  const kiss = emojiRow(row("\u{1F469}\u{200D}\u{2764}\u{FE0F}\u{200D}\u{1F48B}\u{200D}\u{1F468}"));
  assert.equal(withSkinTone(kiss.emoji, kiss.skin, 2, EMOJI_TONES), `\u{1F469}${tone(2)}\u{200D}\u{2764}\u{FE0F}\u{200D}\u{1F48B}\u{200D}\u{1F468}${tone(2)}`, "the heart keeps its own selector");
  const fire = emojiRow(row("\u{1F525}"));
  assert.equal(withSkinTone(fire.emoji, fire.skin, 4, EMOJI_TONES), fire.emoji);
});

test("search finds emoji by shortcode, name and keyword, best match first", () => {
  assert.equal(first("thumbsup"), "\u{1F44D}");
  assert.equal(first("+1"), "\u{1F44D}");
  assert.equal(first(":fire:"), "\u{1F525}");
  assert.equal(first("thumbs_up"), "\u{1F44D}");
  assert.equal(first("pizza"), "\u{1F355}");
  assert.ok(searchEmoji(EMOJI_DATA, "happy").length > 5, "keywords count too");
  assert.deepEqual(searchEmoji(EMOJI_DATA, "zzzzqq"), []);
  assert.ok(searchEmoji(EMOJI_DATA, "face", { maxVersion: 1 }).every(hit => hit.entry.version <= 1));
  assert.ok(searchEmoji(EMOJI_DATA, "face").some(hit => hit.entry.version > 1));
});

test("emoji newer than the device can draw stay out of the categories", () => {
  const sections = emojiByGroup(EMOJI_DATA, EMOJI_GROUPS, { maxVersion: 12 });
  assert.ok(sections.flatMap(s => s.items).every(item => item.entry.version <= 12));
  assert.ok(emojiByGroup(EMOJI_DATA, EMOJI_GROUPS, { hidden: ["flags"] }).every(s => s.key !== "flags"));
});

test("the footer names an emoji by its shortcodes", () => {
  assert.equal(emojiCodesLabel(emojiRow(row("\u{1F44D}"))), ":+1: :thumbsup: :yes:");
  assert.equal(emojiCodesLabel({ codes: [], label: "odd one" }), ":odd_one:");
});

test("frequently used emoji rank by use, then by most recent", () => {
  let store = {};
  store = recordEmojiUse(store, "a", 1);
  store = recordEmojiUse(store, "b", 2);
  store = recordEmojiUse(store, "b", 3);
  store = recordEmojiUse(store, "c", 4);
  assert.deepEqual(frequentEmoji(store, () => true), ["b", "c", "a"]);
  assert.deepEqual(frequentEmoji(store, key => key !== "c"), ["b", "a"], "emoji that cannot be used here are skipped");
  assert.deepEqual(frequentEmoji(store, () => true, 1), ["b"]);
});

test("the frequent list stays small and survives junk in storage", () => {
  let store = {};
  for (let i = 0; i < FREQUENT_KEEP + 20; i++) store = recordEmojiUse(store, `e${i}`, i);
  assert.equal(Object.keys(store).length, FREQUENT_KEEP);
  assert.ok(store[`e${FREQUENT_KEEP + 19}`], "the newest pick is kept");
  assert.deepEqual(readFrequentEmoji("not json"), {});
  assert.deepEqual(readFrequentEmoji("[1,2]"), {});
  assert.deepEqual(readFrequentEmoji(JSON.stringify({ ok: { n: 2, t: 5 }, bad: { n: "x" }, worse: null })), { ok: { n: 2, t: 5 } });
});

test("the newest emoji version the device draws decides what the picker shows", () => {
  const probes = emojiVersionProbes(EMOJI_DATA);
  assert.ok(probes.length > 10);
  assert.deepEqual(probes.map(p => p.version), [...probes.map(p => p.version)].sort((a, b) => b - a));
  const drawsUpTo = max => emoji => probes.find(p => p.emoji === emoji).version <= max;
  assert.equal(highestSupportedVersion(probes, drawsUpTo(15)), 15);
  assert.equal(highestSupportedVersion(probes, () => false), Infinity, "with no colour emoji at all nothing is hidden");
  assert.equal(highestSupportedVersion([], () => true), Infinity);
});

test("a leading emoji splits from a channel name and joins back the same way", () => {
  assert.deepEqual(splitLeadingEmoji("\u{1F3AE}-gaming"), { lead: "\u{1F3AE}", separator: "-", rest: "gaming" });
  assert.deepEqual(splitLeadingEmoji("\u{1F3AE}gaming"), { lead: "\u{1F3AE}", separator: "", rest: "gaming" });
  assert.deepEqual(splitLeadingEmoji("\u{1F44D}\u{1F3FD} Hangout"), { lead: "\u{1F44D}\u{1F3FD}", separator: " ", rest: "Hangout" });
  assert.deepEqual(splitLeadingEmoji("\u{1F1FA}\u{1F1F8}-usa"), { lead: "\u{1F1FA}\u{1F1F8}", separator: "-", rest: "usa" });
  assert.deepEqual(splitLeadingEmoji(":ferris:-rust", name => name === "ferris"), { lead: ":ferris:", separator: "-", rest: "rust" });
  assert.deepEqual(splitLeadingEmoji(":nope:-rust", () => false), { lead: "", separator: null, rest: ":nope:-rust" });
  assert.deepEqual(splitLeadingEmoji("1-general"), { lead: "", separator: null, rest: "1-general" });
  assert.deepEqual(splitLeadingEmoji("lobby"), { lead: "", separator: null, rest: "lobby" });
  assert.equal(joinLeadingEmoji("\u{1F3AE}", "gaming", ""), "\u{1F3AE}gaming");
  assert.equal(joinLeadingEmoji("\u{1F3AE}", "gaming"), "\u{1F3AE}-gaming");
  assert.equal(joinLeadingEmoji("\u{1F3AE}", ""), "\u{1F3AE}");
  assert.equal(joinLeadingEmoji("", "gaming"), "gaming");
  for (const name of ["\u{1F3AE}-gaming", "\u{1F3AE}gaming", "\u{1F3AE} Hang out", "plain"]) {
    const { lead, separator, rest } = splitLeadingEmoji(name);
    assert.equal(joinLeadingEmoji(lead, rest, separator ?? "-"), name);
  }
});
