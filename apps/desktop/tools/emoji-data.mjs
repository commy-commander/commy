import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const GROUPS = [
  { key: "smileys-emotion", name: "Smileys & Emotion" },
  { key: "people-body", name: "People & Body" },
  { key: "animals-nature", name: "Animals & Nature" },
  { key: "food-drink", name: "Food & Drink" },
  { key: "travel-places", name: "Travel & Places" },
  { key: "activities", name: "Activities" },
  { key: "objects", name: "Objects" },
  { key: "symbols", name: "Symbols" },
  { key: "flags", name: "Flags" },
];
const TONES = ["1F3FB", "1F3FC", "1F3FD", "1F3FE", "1F3FF"];
const { SKIN_FIRST, SKIN_EVERY_PERSON, withSkinTone } = createRequire(import.meta.url)("../ui/scripts/emoji-picker.js");
const COMPONENT_GROUP = 2;

const pickable = entry => typeof entry.group === "number" && entry.group !== COMPONENT_GROUP;
const groupIndex = group => (group > COMPONENT_GROUP ? group - 1 : group);

const fromHex = hex => String.fromCodePoint(...hex.split("-").map(h => parseInt(h, 16)));

function skinRule(entry) {
  const single = (entry.skins || []).filter(s => typeof s.tone === "number");
  if (single.length !== TONES.length) return 0;
  const tones = TONES.map(fromHex);
  const fits = rule => single.every(s => withSkinTone(entry.emoji, rule, s.tone, tones) === s.emoji);
  if (fits(SKIN_FIRST)) return SKIN_FIRST;
  if (fits(SKIN_EVERY_PERSON)) return SKIN_EVERY_PERSON;
  return 0;
}

function shortcodesFor(hexcode, sources) {
  const seen = new Set();
  for (const source of sources) for (const code of [].concat(source[hexcode] || [])) seen.add(code.toLowerCase());
  return [...seen];
}

function keywordsFor(entry) {
  const words = new Set(entry.label.toLowerCase().split(/[\s:,]+/));
  return [...new Set((entry.tags || []).map(t => t.toLowerCase()))].filter(t => !words.has(t));
}

export function buildEmojiData(dir) {
  const read = path => JSON.parse(readFileSync(join(dir, path), "utf8"));
  const pkg = read("package.json");
  const data = read("en/data.json");
  const sources = [read("en/shortcodes/iamcal.json"), read("en/shortcodes/emojibase.json")];
  const rows = data
    .filter(pickable)
    .sort((a, b) => a.order - b.order)
    .map(e => [
      e.emoji,
      e.label.replace(/^flag: /, "flag "),
      shortcodesFor(e.hexcode, sources).join(" "),
      keywordsFor(e).join(" "),
      groupIndex(e.group),
      e.version,
      skinRule(e),
    ]);
  return { version: pkg.version, rows };
}

export function renderEmojiScript({ version, rows }) {
  return `/*
 * @license
 * Emoji list, names, keywords and shortcodes from emojibase-data ${version}
 * Copyright (c) 2017-2019 Miles Johnson
 * SPDX-License-Identifier: MIT
 */
"use strict";

const EMOJI_TONES = ${JSON.stringify(TONES.map(fromHex))};
const EMOJI_GROUPS = ${JSON.stringify(GROUPS)};
const EMOJI_DATA = [
${rows.map(row => JSON.stringify(row)).join(",\n")}
];

if (typeof module !== "undefined" && module.exports) {
  module.exports = { EMOJI_TONES, EMOJI_GROUPS, EMOJI_DATA };
}
`;
}

if (process.argv[1] === import.meta.filename) {
  const dir = process.argv[2];
  if (!dir) {
    console.error("usage: node apps/desktop/tools/emoji-data.mjs <unpacked emojibase-data package>");
    process.exit(1);
  }
  const out = join(import.meta.dirname, "..", "ui", "scripts", "emoji.js");
  const built = buildEmojiData(dir);
  writeFileSync(out, renderEmojiScript(built));
  console.log(`${built.rows.length} emoji from emojibase-data ${built.version} written to ${out}`);
}
