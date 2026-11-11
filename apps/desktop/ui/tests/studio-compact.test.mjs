import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { windowFit } = require("../scripts/look.js");

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio-compact.css");
const C = 'html[data-look="studio"][data-fit="compact"]';
const selectors = [...css.replace(/@media[^{]+\{/g, "").matchAll(/(^|\})\s*([^{}@]+)\{/g)]
  .flatMap(match => match[2].split(/,\n/).map(part => part.trim()).filter(Boolean));
const block = query => {
  const start = css.indexOf(`@media ${query} {`);
  assert.ok(start >= 0, query);
  return css.slice(start, css.indexOf("\n}\n", start));
};
const rule = (selector, within = css) => {
  const start = within.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return within.slice(start, within.indexOf("}", start));
};

test("the compact sheet loads after the forum sheet, compact only, from the tokens", () => {
  const anchor = '  <link rel="stylesheet" id="studio-forum" href="styles/studio-forum.css" media="not all" />\n';
  const link = '  <link rel="stylesheet" id="studio-compact" href="styles/studio-compact.css" media="not all" />\n';
  assert.ok(html.includes(anchor + link));
  assert.equal(html.split('id="studio-compact"').length, 2);
  assert.ok(selectors.length > 15);
  for (const selector of selectors) assert.ok(selector.startsWith(C), selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
  assert.equal(html.includes("scripts/studio-compact.js"), false);
});

test("the compact sheet uses the guide's own width breakpoints", () => {
  assert.deepEqual([...css.matchAll(/@media ([^{]+) \{/g)].map(m => m[1]), ["(min-width: 1180px)", "(max-width: 1179px)", "(max-width: 1040px)"]);
  assert.equal(windowFit(1000, 650), "compact");
  assert.equal(windowFit(1100, 700), "compact");
  assert.equal(windowFit(1300, 700), "compact");
  assert.equal(windowFit(1200, 800), "roomy");
  assert.equal(windowFit(1440, 900), "roomy");
});

test("the top bar is 48 high at 6, the window starts at 60 and the channel column is 248", () => {
  const app = rule(`${C} .app`);
  assert.match(app, /grid-template-rows: 48px minmax\(0, 1fr\);/);
  assert.match(app, /gap: 6px 0;/);
  assert.match(app, /padding-top: 6px;/);
  assert.match(app, /--sidebar-w: 248px;/);
  assert.match(rule(`${C} .user-panel`), /height: 46px;/);
  assert.ok(css.includes(`${C} .user-panel { padding-right: 52px; }`));
  assert.ok(css.includes(`${C} .user-actions { right: 6px; }`));
  assert.ok(css.includes(`${C} .settings-layer { top: 60px; }`));
  assert.ok(css.includes(`${C} .main > .composer { margin: 0 20px 18px; }`));
  assert.ok(block("(min-width: 1180px)").includes(`${C} { --st-wing: clamp(232px, calc(9.2vw + 123px), 300px); }`));
});

test("under 1180 the account is the face and gear, the chat bar keeps emoji, and search is a round key that opens", () => {
  const narrow = block("(max-width: 1179px)");
  assert.ok(narrow.includes(`${C} .user-chip .user-meta { display: none; }`));
  assert.ok(narrow.includes(`${C} .main > .composer #text-style-btn,\n  ${C} .main > .composer #gif-btn { display: none; }`));
  const key = rule(`${C} .chat-header .search-box:not(:focus-within, .open)`, narrow);
  assert.match(key, /flex: 0 0 38px;/);
  assert.match(key, /width: 38px;/);
  const input = rule(`${C} .chat-header .search-box:not(:focus-within, .open) input`, narrow);
  assert.match(input, /opacity: 0;/);
  assert.doesNotMatch(input, /display: none/);
  const open = rule(`${C} .chat-header .search-box:is(:focus-within, .open)`, narrow);
  assert.match(open, /position: absolute;/);
  assert.match(open, /width: max\(100%, 118px\);/);
  assert.match(rule(`${C} .chat-header .header-actions:has(> .search-box:is(:focus-within, .open))::after`, narrow), /flex: 0 0 38px;/);
});

test("under 1040 the second and third header tools give way", () => {
  assert.ok(block("(max-width: 1040px)").includes(`${C} .header-actions > #threads-btn,\n  ${C} .header-actions > #saved-btn { display: none; }`));
  assert.ok(html.indexOf('id="threads-btn"') < html.indexOf('id="saved-btn"'));
  assert.ok(html.indexOf('id="pins-btn"') < html.indexOf('id="threads-btn"'));
  assert.ok(html.indexOf('id="saved-btn"') < html.indexOf('id="members-btn"'));
});
