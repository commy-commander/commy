import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  SERVER_TAG_COLORS, SERVER_TAG_EFFECTS, NO_SERVER_TAG,
  cleanTagLetters, serverTagTitle, wearableTags, wornServerId, serverTagDraft,
} = require("../scripts/server-tags.js");

const ui = join(import.meta.dirname, "..");
const css = readFileSync(join(ui, "styles", "styles.css"), "utf8");
const core = readFileSync(join(ui, "..", "..", "..", "crates", "commy-core", "src", "server_tag.rs"), "utf8");

function luminance(hex) {
  const n = parseInt(hex.slice(1), 16);
  const [r, g, b] = [n >> 16, (n >> 8) & 255, n & 255].map(v => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

test("tag letters become up to four capital letters or digits as you type", () => {
  assert.equal(cleanTagLetters("gg"), "GG");
  assert.equal(cleanTagLetters("a b-c!d"), "ABCD");
  assert.equal(cleanTagLetters("rockets"), "ROCK");
  assert.equal(cleanTagLetters("été"), "ÉTÉ");
  assert.equal(cleanTagLetters("\u{1f680}"), "");
  assert.equal(cleanTagLetters(undefined), "");
});

test("only servers with a tag can be worn, and groups never", () => {
  const servers = [
    { id: "b", name: "Zebra Club", tag: { letters: "ZC" } },
    { id: "a", name: "Astro", tag: { letters: "AST" } },
    { id: "c", name: "Plain" },
    { id: "d", name: "Group chat", group: true, tag: { letters: "GC" } },
  ];
  assert.deepEqual(wearableTags(servers), [{ value: "a", label: "Astro (AST)" }, { value: "b", label: "Zebra Club (ZC)" }]);
  assert.deepEqual(wearableTags(undefined), []);
  assert.equal(wornServerId({ server_tag: { server: "a" } }), "a");
  assert.equal(wornServerId({}), NO_SERVER_TAG);
  assert.equal(serverTagTitle({ server: "Astro" }), "Astro server tag");
});

test("the editor starts from the saved tag or sensible defaults", () => {
  assert.deepEqual(serverTagDraft(null), { letters: "", emoji: "", color: "blue", effect: "none" });
  assert.deepEqual(serverTagDraft({ letters: "GG", emoji: "\u{1f680}", color: "pink", effect: "glow" }), { letters: "GG", emoji: "\u{1f680}", color: "pink", effect: "glow" });
  assert.equal(serverTagDraft({ letters: "GG", color: "neon", effect: "spin" }).color, "blue");
});

test("the colours and effects match the app's rules and each has a look", () => {
  for (const color of SERVER_TAG_COLORS) {
    assert.ok(core.includes(`"${color}"`), `${color} is not allowed by the app`);
    const look = new RegExp(`\\.st-${color} \\{ --st-bg: (#[0-9a-f]{6}); --st-ink: (#[0-9a-f]{6}); \\}`).exec(css);
    assert.ok(look, `${color} has no look`);
    assert.ok(contrast(look[1], look[2]) >= 4.5, `${color} letters are hard to read`);
  }
  for (const { value } of SERVER_TAG_EFFECTS) {
    assert.ok(core.includes(`"${value}"`), `${value} is not allowed by the app`);
    if (value !== "none") assert.match(css, new RegExp(`\\.server-tag\\.st-fx-${value}`));
  }
});
