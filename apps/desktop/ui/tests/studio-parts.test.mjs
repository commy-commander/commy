import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const SHEET = readFileSync(new URL("../styles/studio-parts.css", import.meta.url), "utf8");
const SWITCH = 'html[data-look="studio"]';
const AT_RULE_FRAMES = /@keyframes[^{]+\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g;
const MASK_INK = "#000";
const LITE_FIELD = "#272a34";

function styleSelectors(css) {
  const flat = css.replace(AT_RULE_FRAMES, "").replace(/@(media|container)[^{]+\{/g, "");
  return [...flat.matchAll(/(^|\})\s*([^{}@]+)\{/g)].flatMap(match => match[2].split(",").map(part => part.trim())).filter(Boolean);
}

test("every shared part stays hidden until the new look is switched on", () => {
  const selectors = styleSelectors(SHEET);
  assert.ok(selectors.length > 80);
  for (const selector of selectors) assert.ok(selector.startsWith(SWITCH), selector);
});

test("the shared parts take their colours from the tokens", () => {
  const withoutGrain = SHEET.replace(/--grain:[^;]+;/, "");
  const literals = [...withoutGrain.matchAll(/#[0-9a-fA-F]{3,8}\b/g)].map(match => match[0]);
  assert.deepEqual([...new Set(literals)].sort(), [MASK_INK, LITE_FIELD].sort());
  assert.doesNotMatch(withoutGrain, /rgba?\(\s*\d/);
});

test("the shared parts name the classes the phone and desktop layouts build on", () => {
  for (const part of [".key", ".key.sm", ".key.tap", ".key.lg", ".key.sq", ".key.primary", ".key.cream", ".key.danger",
    ".key.is-down", ".key.latch.is-down", ".key.muted", ".key.is-off", ".slab", ".slab.thin", ".led", ".i",
    "form.bar", "form.bar .plus", "form.bar .slot", "form.bar .cfield", "form.bar .copts", "form.bar .opt-k", "form.bar .send"]) {
    assert.ok(SHEET.includes(`${SWITCH} ${part} {`) || SHEET.includes(`${SWITCH} ${part},`), part);
  }
});

test("a key never drops below its neighbours when it latches, mutes or switches off", () => {
  const moved = [...SHEET.matchAll(/([^{}]+)\{[^{}]*\btranslate:[^;]+;[^{}]*\}/g)].map(match => match[1].trim());
  for (const selector of moved) assert.match(selector, /form\.bar(\.is-open)? \.opt-k/, selector);
});

test("the sheet carries no comments", () => {
  assert.doesNotMatch(SHEET, /\/\*/);
});
