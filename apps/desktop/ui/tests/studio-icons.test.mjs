import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";

const require = createRequire(import.meta.url);
const { STUDIO_ICON_PATHS } = require("../vendor/phosphor/icons.js");
const { STUDIO_ICON_GLYPHS, StudioIcons, studioIconKnown, studioIconMarkup, paintStudioIcons } = require("../scripts/studio-icons.js");

const ICON_MAP = JSON.parse(readFileSync(new URL("../../tools/studio-icon-map.json", import.meta.url), "utf8"));
const SAMPLE = { mic: ["<path d=\"M1\"/>", "<path d=\"M2\"/>"], plus: ["<path d=\"M3\"/>", "<path d=\"M4\"/>"] };

function fakePlaceholder(name, className = "") {
  return { className, outerHTML: "", getAttribute: attr => (attr === "data-i" ? name : null) };
}

function fakeRoot(placeholders) {
  return { querySelectorAll: selector => (selector === "i[data-i]" ? placeholders : []) };
}

test("every icon the app names has a regular and a filled drawing", () => {
  assert.deepEqual(Object.keys(STUDIO_ICON_PATHS).sort(), Object.keys(ICON_MAP).sort());
  for (const [name, pair] of Object.entries(STUDIO_ICON_PATHS)) {
    assert.equal(pair.length, 2, name);
    assert.match(pair[0], /^<(path|circle|rect|line|polyline|polygon|ellipse|g)\b/, name);
    assert.match(pair[1], /^<(path|circle|rect|line|polyline|polygon|ellipse|g)\b/, name);
    assert.doesNotMatch(pair.join(""), /<script|on[a-z]+=|href=/i, name);
  }
});

test("an icon carries both drawings so a stylesheet picks rest or on without a redraw", () => {
  const svg = studioIconMarkup("mic", "", SAMPLE);
  assert.equal(svg, "<svg class=\"i\" data-icon=\"mic\" viewBox=\"0 0 256 256\" aria-hidden=\"true\" focusable=\"false\"><g class=\"i-rest\"><path d=\"M1\"/></g><g class=\"i-on\"><path d=\"M2\"/></g></svg>");
});

test("a plain glyph has one drawing and is marked so a key can thicken it", () => {
  const svg = studioIconMarkup("plus", "", SAMPLE);
  assert.match(svg, /^<svg class="i i-g" data-icon="plus"/);
  assert.match(svg, /><path d="M3"\/><\/svg>$/);
  assert.doesNotMatch(svg, /i-on|M4/);
  for (const glyph of STUDIO_ICON_GLYPHS) assert.ok(studioIconKnown(glyph, STUDIO_ICON_PATHS), glyph);
});

test("unknown or unsafe names draw nothing", () => {
  assert.equal(studioIconMarkup("no-such-icon", "", SAMPLE), "");
  assert.equal(studioIconMarkup("constructor", "", SAMPLE), "");
  assert.equal(studioIconMarkup("mic\"><script>", "", SAMPLE), "");
  assert.equal(studioIconMarkup(undefined, "", SAMPLE), "");
  assert.equal(studioIconKnown("toString", SAMPLE), false);
});

test("extra classes from the placeholder are kept, anything unsafe is dropped", () => {
  assert.match(studioIconMarkup("mic", "lead on", SAMPLE), /^<svg class="i lead on" /);
  assert.match(studioIconMarkup("mic", "ok \"><b> i i-g", SAMPLE), /^<svg class="i ok" /);
  assert.match(studioIconMarkup("plus", "big", SAMPLE), /^<svg class="i i-g big" /);
});

test("painting swaps each known placeholder for its icon and leaves unknown ones alone", () => {
  const mic = fakePlaceholder("mic", "lead");
  const unknown = fakePlaceholder("no-such-icon");
  const painted = paintStudioIcons(fakeRoot([mic, unknown]), SAMPLE);
  assert.equal(painted, 1);
  assert.match(mic.outerHTML, /^<svg class="i lead" data-icon="mic"/);
  assert.equal(unknown.outerHTML, "");
  assert.equal(paintStudioIcons(null, SAMPLE), 0);
  assert.equal(StudioIcons.paint, paintStudioIcons);
});
