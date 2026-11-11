import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { discoverEmptyLook, discoverKeyNames } = require("../scripts/studio-discover.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const app = read("../scripts/app.js");
const glue = read("../scripts/studio-discover.js");
const css = read("../styles/studio-discover.css");
const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const SLIM = 'html[data-look="studio"][data-fit="slim"]';
const selectors = [...css.replace(/@keyframes[^{]+\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, "").replace(/@media[^{]+\{/g, "").matchAll(/(^|\})\s*([^{}@]+)\{/g)]
  .flatMap(match => match[2].split(/,\n/).map(part => part.trim()).filter(Boolean));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("discover loads with the new look, roomy only, from the tokens", () => {
  const link = '<link rel="stylesheet" id="studio-discover" href="styles/studio-discover.css" media="not all" />';
  assert.ok(html.indexOf(link) > html.indexOf('<link rel="stylesheet" id="studio-float" href="styles/studio-float.css" media="not all" />'));
  assert.ok(html.includes('<script src="scripts/overlay-view.js"></script>\n  <script src="scripts/studio-discover.js"></script>'));
  assert.ok(app.includes('  window.addEventListener("resize", applyMembersLayout);\n  if (studioLook()) studioDiscover();\n'));
  assert.ok(selectors.length > 25);
  for (const selector of selectors) assert.ok(selector.startsWith(R) || selector === `${SLIM} .discover-star`, selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
  assert.doesNotMatch(glue, /\/\/|\/\*/);
});

test("discover stands in the chat's place with a big title and a capsule search", () => {
  assert.match(rule(`${R} .discover`), /background: none;/);
  assert.match(rule(`${R} .discover-head`), /padding: 24px 22px 0;/);
  assert.match(rule(`${R} .discover-head`), /gap: 20px;/);
  assert.match(rule(`${R} .discover-title h2`), /font: 700 28px\/1\.1 var\(--font-display\);/);
  assert.match(rule(`${R} .discover-title p`), /font: 400 14px\/1\.35 var\(--font\);/);
  assert.match(rule(`${R} .discover-search`), /flex: 0 0 300px;/);
  assert.match(rule(`${R} .discover-search`), /height: 42px;/);
  assert.match(rule(`${R} .discover-search`), /border-radius: var\(--radius-pill\);/);
  assert.match(rule(`${R} .discover-search:focus-within`), /0 0 0 2px var\(--accent\);/);
  assert.match(css, /visibility: hidden;/);
});

test("category chips are ringed when picked and sit over a hairline", () => {
  assert.match(rule(`${R} .discover-chips`), /margin: 18px 22px 0;/);
  assert.match(rule(`${R} .discover-chips`), /padding: 0 0 18px;/);
  assert.match(rule(`${R} .discover-chip`), /height: 32px;/);
  assert.match(rule(`${R} .discover-chip`), /font: 500 13\.5px\/1 var\(--font\);/);
  assert.match(css, /\.discover-chip\[aria-pressed="true"\]:hover \{\n  background: var\(--bg-active\);\n  box-shadow: inset 0 0 0 1\.5px var\(--accent-lit\);/);
});

test("cards go three across with a glass Join key", () => {
  assert.match(rule(`${R} .discover-grid`), /grid-template-columns: repeat\(3, minmax\(0, 1fr\)\);/);
  assert.match(rule(`${R} .discover-grid`), /gap: 12px;/);
  assert.match(rule(`${R} .discover-card`), /min-height: 196px;/);
  assert.match(rule(`${R} .discover-card`), /border-radius: 20px;/);
  assert.match(rule(`${R} .discover-card`), /padding: 18px 18px 16px;/);
  assert.match(rule(`${R} .discover-card:hover`), /translate: 0 -2px;/);
  assert.match(rule(`${R} .discover-card-name`), /font: 700 18px\/1\.2 var\(--font-display\);/);
  assert.match(rule(`${R} .discover-card-text`), /font: 400 14px\/1\.45 var\(--font\);/);
  assert.match(rule(`${R} .discover .key.sm`), /padding: 0 18px;/);
  assert.deepEqual(discoverKeyNames(true), { key: true, sm: true });
  assert.deepEqual(discoverKeyNames(false), { key: false, sm: false });
});

test("empty states show the quiet well or the quiet star, errors in red words", () => {
  assert.deepEqual(discoverEmptyLook("search"), { art: "well", tone: "" });
  assert.deepEqual(discoverEmptyLook("clock"), { art: "star", tone: "" });
  assert.deepEqual(discoverEmptyLook("triangle-alert"), { art: "star", tone: "error" });
  assert.deepEqual(discoverEmptyLook(""), { art: "well", tone: "" });
  assert.match(rule(`${R} .discover-empty`), /padding: 34px 18px 26px;/);
  assert.match(rule(`${R} .discover-empty-art`), /width: 64px;/);
  assert.match(rule(`${R} .discover-star`), /--star-size: 56px;/);
  assert.match(rule(`${R} .discover-empty h3`), /font: 700 17px\/1\.2 var\(--font-display\);/);
  assert.match(rule(`${R} .discover-empty[data-tone="error"] p`), /color: var\(--danger-text\);/);
  assert.match(rule(`${SLIM} .discover-star`), /display: none;/);
});
