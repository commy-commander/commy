import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { LABELS_TIMING, LABELS_SPACE, LABELS_SIDES, labelsText, labelsPlace, labelsFits } = require("../scripts/studio-labels.js");

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const app = read("../scripts/app.js");
const glue = read("../scripts/studio-labels.js");
const css = read("../styles/studio-labels.css");
const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const view = { width: 1280, height: 800 };
const size = { width: 80, height: 30 };
const box = (left, top, width = 32, height = 32) => ({ left, top, width, height, right: left + width, bottom: top + height });

test("a label comes from its tip, then its title, then the aria label of an icon-only control", () => {
  assert.equal(labelsText({ tip: "Tip", title: "Title", aria: "Aria" }), "Tip");
  assert.equal(labelsText({ title: " Threads ", aria: "Other" }), "Threads");
  assert.equal(labelsText({ title: "Set status", text: "Commy Commander" }), "Set status");
  assert.equal(labelsText({ aria: "Minimize", text: "" }), "Minimize");
  assert.equal(labelsText({ aria: "Accept", text: "Accept" }), "");
  assert.equal(labelsText({ aria: "Message", typing: true }), "");
  assert.equal(labelsText({ title: "   " }), "");
  assert.equal(labelsText({}), "");
});

test("labels sit above the control and flip below when the window edge is too close", () => {
  const above = labelsPlace(box(600, 400), size, view);
  assert.equal(above.side, "top");
  assert.equal(above.top, 400 - LABELS_SPACE.gap - size.height);
  assert.equal(above.left, 600 + 16 - 40);
  assert.equal(above.arrow, 40);
  const below = labelsPlace(box(600, 10), size, view);
  assert.equal(below.side, "bottom");
  assert.equal(below.top, 10 + 32 + LABELS_SPACE.gap);
});

test("labels stay inside the window and the arrow still points at the control", () => {
  const corner = labelsPlace(box(1240, 10), size, view);
  assert.equal(corner.left, view.width - LABELS_SPACE.edge - size.width);
  assert.equal(corner.arrow, 1256 - corner.left);
  const leftEdge = labelsPlace(box(0, 400, 10, 32), size, view);
  assert.equal(leftEdge.left, LABELS_SPACE.edge);
  assert.equal(leftEdge.arrow, LABELS_SPACE.arrowInset);
});

test("a control can ask for a side, and an unknown side falls back to the top", () => {
  assert.equal(labelsPlace(box(600, 400), size, view, "right").side, "right");
  assert.equal(labelsPlace(box(600, 400), size, view, "left").side, "left");
  assert.equal(labelsPlace(box(600, 400), size, view, "sideways").side, "top");
  assert.deepEqual(LABELS_SIDES, ["top", "bottom", "right", "left"]);
  assert.equal(labelsFits("left", box(20, 400), size, view), false);
});

test("labels wait a moment on hover, then show at once while moving between controls", () => {
  assert.ok(LABELS_TIMING.restMs >= 250 && LABELS_TIMING.restMs <= 500);
  assert.ok(LABELS_TIMING.warmMs > 0);
  assert.match(glue, /if \(warm \|\| wait === 0\) labelsShow\(node\);/);
  assert.match(glue, /if \(e\.pointerType === "touch" \|\| labelsSlim\(\)\) return;/);
  assert.match(glue, /node\.matches\(":focus-visible"\)/);
});

test("the system tooltip steps aside while ours shows, and the title comes back after", () => {
  assert.match(glue, /node\.removeAttribute\("title"\);/);
  assert.match(glue, /if \(!node\.hasAttribute\("title"\)\) node\.setAttribute\("title", held\.title\);/);
  assert.match(glue, /if \(!named && !labelsVisibleText\(node\)\.trim\(\)\) node\.setAttribute\("aria-label", title\);/);
  assert.match(glue, /labelsNode\.setAttribute\("popover", "manual"\)/);
  assert.doesNotMatch(read("../scripts/studio-minors.js"), /button\[title=/);
  assert.match(read("../scripts/studio-phone.js"), /button\[aria-label="Reply"\]/);
});

test("the labels load with the new look, start once, and draw from the tokens", () => {
  const sheet = html.indexOf('<link rel="stylesheet" id="studio-labels" href="styles/studio-labels.css" media="not all" />');
  assert.ok(sheet > html.indexOf('id="studio-pops"'));
  assert.ok(sheet < html.indexOf('id="studio-phone"'));
  assert.ok(html.indexOf('<script src="scripts/studio-labels.js"></script>') > html.indexOf('<script src="scripts/look.js"></script>'));
  assert.equal(app.split("  if (studioLook()) studioLabels();\n").length, 2);
  const selectors = [...css.replace(/@starting-style\s*\{/g, "").matchAll(/(^|\})\s*([^{}@]+)\{/g)].map(match => match[2].trim());
  assert.ok(selectors.length >= 8);
  for (const selector of selectors) assert.ok(selector.startsWith('html[data-look="studio"]'), selector);
  assert.match(css, new RegExp(`${R.replace(/[[\]().:"]/g, "\\$&")} \\.studio-label \\{[^}]*background: var\\(--bg-raised\\);`));
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
  assert.equal(/\/\/|\/\*/.test(glue.replace(/"[^"]*"|`[^`]*`/g, "")), false);
});
