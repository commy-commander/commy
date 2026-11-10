import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";

const require = createRequire(import.meta.url);
const { STUDIO_ICON_PATHS } = require("../vendor/phosphor/icons.js");
const { STUDIO_ICON_GLYPHS, STUDIO_APP_ICONS, STUDIO_STATIC_ICONS, StudioIcons, studioIconKnown, studioIconMarkup, paintStudioIcons, studioAppIconName, studioAppIconOn, studioStaticIconName, studioIconNode, swapStaticStudioIcons } = require("../scripts/studio-icons.js");

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

function fakeSvgDocument() {
  return {
    createElement: () => ({
      set innerHTML(markup) {
        const attrs = {};
        const style = {};
        this.content = { firstElementChild: markup ? { markup, attrs, style, setAttribute(name, value) { attrs[name] = value; } } : null };
      },
    }),
  };
}

function fakeStaticIcon(d, width, doc, extraChildren = 0) {
  return {
    childElementCount: 1 + extraChildren,
    firstElementChild: { getAttribute: name => (name === "d" ? d : null) },
    ownerDocument: doc,
    replaced: null,
    getAttribute: name => (name === "width" ? width : null),
    replaceWith(node) { this.replaced = node; },
  };
}

test("every app icon name maps to a drawing that exists", () => {
  for (const [name, mapped] of Object.entries(STUDIO_APP_ICONS)) {
    assert.ok(studioIconKnown(mapped, STUDIO_ICON_PATHS), `${name} > ${mapped}`);
    assert.equal(studioAppIconName(name, STUDIO_ICON_PATHS), mapped);
  }
  assert.equal(studioAppIconName("steam", STUDIO_ICON_PATHS), "");
  assert.equal(studioAppIconName("constructor", STUDIO_ICON_PATHS), "");
  assert.equal(studioAppIconName(undefined, STUDIO_ICON_PATHS), "");
  assert.equal(studioAppIconName("mic", SAMPLE), "mic");
  assert.equal(studioAppIconName("gear", SAMPLE), "");
});

test("a pinned pin and a saved bookmark are drawn filled", () => {
  assert.equal(studioAppIconOn("pin"), true);
  assert.equal(studioAppIconOn("bookmarkOn"), true);
  assert.equal(studioAppIconOn("bookmark"), false);
  assert.equal(studioAppIconOn("pinOutline"), false);
});

test("each icon drawn into the page is recognised by the start of its path, and no start hides another", () => {
  const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
  const drawn = [...html.matchAll(/<path[^>]* d="([^"]+)"/g)].map(match => match[1]);
  STUDIO_STATIC_ICONS.forEach(([start, name], index) => {
    assert.ok(studioIconKnown(name, STUDIO_ICON_PATHS), name);
    assert.ok(drawn.some(d => d.startsWith(start)), start);
    for (const [earlier] of STUDIO_STATIC_ICONS.slice(0, index)) assert.equal(start.startsWith(earlier), false, start);
    assert.equal(studioStaticIconName(`${start} and the rest`, STUDIO_ICON_PATHS), name);
  });
  assert.equal(studioStaticIconName("M0 0h24v24H0z", STUDIO_ICON_PATHS), "");
  assert.equal(studioStaticIconName(null, STUDIO_ICON_PATHS), "");
});

test("an app icon keeps the size it was asked for and can be drawn filled", () => {
  globalThis.STUDIO_ICON_PATHS = STUDIO_ICON_PATHS;
  try {
    const doc = fakeSvgDocument();
    const rest = studioIconNode("mic", 18, false, doc);
    assert.match(rest.markup, /^<svg class="i ai" data-icon="mic"/);
    assert.deepEqual(rest.attrs, { width: 18, height: 18 });
    assert.deepEqual(rest.style, { width: "18px", height: "18px" });
    assert.match(studioIconNode("bookmark", 20, true, doc).markup, /^<svg class="i ai on" /);
    assert.equal(studioIconNode("no-such-icon", 20, false, doc), null);
  } finally {
    delete globalThis.STUDIO_ICON_PATHS;
  }
});

test("only single-path icons the table knows are swapped in the page", () => {
  globalThis.STUDIO_ICON_PATHS = STUDIO_ICON_PATHS;
  try {
    const doc = fakeSvgDocument();
    const gear = fakeStaticIcon("M19.4 13a7.9 7.9 0 0 0 0-2l2-1.6", "18", doc);
    const unknown = fakeStaticIcon("M0 0h24v24H0z", "20", doc);
    const twoPaths = fakeStaticIcon("M19.4 13a7.9 7.9 0 0 0 0-2", "20", doc, 1);
    const unsized = fakeStaticIcon("M11 5h2v6h6v2h-6v6h-2", null, doc);
    const root = { querySelectorAll: selector => (selector === 'svg[viewBox="0 0 24 24"]' ? [gear, unknown, twoPaths, unsized] : []) };
    assert.equal(swapStaticStudioIcons(root), 2);
    assert.match(gear.replaced.markup, /data-icon="settings"/);
    assert.equal(gear.replaced.attrs.width, 18);
    assert.equal(unknown.replaced, null);
    assert.equal(twoPaths.replaced, null);
    assert.match(unsized.replaced.markup, /data-icon="plus"/);
    assert.equal(unsized.replaced.attrs.width, 20);
  } finally {
    delete globalThis.STUDIO_ICON_PATHS;
  }
});
