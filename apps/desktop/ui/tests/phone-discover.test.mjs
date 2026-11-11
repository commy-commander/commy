import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("../styles/phone-discover.css");
const glue = read("../scripts/phone-discover.js");
const html = read("../index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';
const rule = selector => {
  const start = css.indexOf(`${SLIM} ${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};
const has = (selector, lines) => {
  const body = rule(selector);
  for (const line of lines) assert.ok(body.includes(line), `${selector}: ${line}`);
};

test("every Discover phone rule waits for the new look and a slim window", () => {
  const selectors = css.match(/(?:^|\})\s*([^{}@]+)\{/g) || [];
  assert.ok(selectors.length > 40);
  for (const raw of selectors) {
    for (const selector of raw.replace(/^\}/, "").replace(/\{$/, "").replace(/\([^()]*\)/g, "").split(",")) {
      assert.ok(selector.trim().startsWith(SLIM), selector.trim());
    }
  }
});

test("the Discover phone sheet has no hex colours and no comments", () => {
  assert.doesNotMatch(css, /#[0-9a-fA-F]{3,8}\b(?![\w-])/);
  assert.ok(!css.includes("/*"));
  assert.ok(!glue.includes("//") && !glue.includes("/*"));
});

test("the sheet is switched on with the look, right after the phone sheet, and its script runs after the phone glue", () => {
  assert.match(html, /id="studio-phone" href="styles\/phone-touch\.css" media="not all" \/>\n(\s*<link rel="stylesheet" id="studio-phone-[a-z-]+" href="styles\/phone-[a-z-]+\.css" media="not all" \/>\n)*\s*<link rel="stylesheet" id="studio-phone-discover" href="styles\/phone-discover\.css" media="not all" \/>/);
  assert.match(html, /<script src="scripts\/studio-phone\.js"><\/script>\n(\s*<script src="scripts\/phone-[a-z-]+\.js"><\/script>\n)*\s*<script src="scripts\/phone-discover\.js"><\/script>/);
});

test("Discover is a pushed screen: the head holds the back key and the title, and one slab holds the rest", () => {
  has("#discover", ["grid-template-columns: 50px minmax(0, 1fr);", "grid-template-rows: 60px auto auto auto minmax(0, 1fr);", "column-gap: 4px;", "background: none;"]);
  has("#discover::before", ["left: 10px;", "right: 10px;", "top: 66px;", "border-radius: var(--radius-xl);", "var(--panel)"]);
  has("#discover-back", ["width: 44px;", "height: 44px;", "color: var(--text-muted);"]);
  has("#discover .discover-title h2", ["font: 700 22px/1 var(--font-display);", "color: var(--header-primary);"]);
  has("#discover-sub", ["font: 400 14.5px/1.45 var(--font);", "color: var(--text-muted);"]);
  assert.ok(css.includes(`${SLIM} .main:has(> #discover:not(.hidden)) > :not(#discover) { visibility: hidden; }`));
});

test("the search is a capsule slot and the categories are chips that scroll and fade at the right", () => {
  has("#discover .discover-search", ["height: 48px;", "padding: 0 16px;", "border-radius: 999px;", "background: var(--input-bg);", "box-shadow: var(--slot);"]);
  has("#discover .discover-search:focus-within", ["0 0 0 2px var(--accent)"]);
  has("#discover-chips", ["gap: 6px;", "mask-image: linear-gradient(90deg, var(--white) 84%, transparent);"]);
  has("#discover-chips .discover-chip", ["height: 40px;", "padding: 0 16px;", "border-radius: 999px;", "font: 500 14.5px/1 var(--font);", "box-shadow: inset 0 0 0 1px var(--line);"]);
  assert.match(css, /discover-chip\[aria-pressed="true"\]:hover \{\n  background: var\(--bg-active\);\n  box-shadow: inset 0 0 0 1\.5px var\(--accent-lit\);/);
});

test("one card a row with the guide's parts, and Join is a small glass capsule, not the bright key", () => {
  has("#discover-grid", ["grid-template-columns: minmax(0, 1fr);", "gap: 10px;"]);
  has("#discover-grid .discover-card", ["padding: 16px;", "border-radius: 20px;", "background: rgb(var(--lamp) / .03);", "box-shadow: inset 0 0 0 1px var(--line);"]);
  has("#discover-grid .discover-card-name", ["font: 700 18px/1.2 var(--font-display);"]);
  has("#discover-grid .discover-card-meta", ["gap: 7px;", "font: 500 12.5px/1.2 var(--font);", "color: var(--text-muted);"]);
  has("#discover-grid .discover-dot", ["width: 3px;", "height: 3px;", "background: var(--text-faint);"]);
  has("#discover-grid .discover-card-text", ["font: 400 14px/1.45 var(--font);"]);
  const key = css.slice(css.indexOf(`${SLIM} #discover-grid .discover-card-foot .btn-primary,`), css.indexOf("}", css.indexOf(`${SLIM} #discover-grid .discover-card-foot .btn-primary,`)));
  for (const line of ["height: 40px;", "padding: 0 20px;", "border-radius: 999px;", "background: var(--k-plain);", "box-shadow: var(--k-plain-shadow);", "font: 600 14px/1 var(--font);", "color: var(--header-primary);"]) assert.ok(key.includes(line), line);
  assert.ok(key.includes(`${SLIM} #discover .discover-retry,`) && key.includes(`${SLIM} #discover .discover-more {`));
  assert.ok(!key.includes("var(--accent)"));
});

test("empty and error states are the quiet well or the quiet star, with the server's error in red words", () => {
  has("#discover-empty-art", ["width: 64px;", "height: 64px;", "border-radius: 50%;", "background: var(--input-bg);", "box-shadow: var(--slot);", "color: var(--text-faint);"]);
  has("#discover-empty-art > .star", ["--star-size: 56px;"]);
  has("#discover-empty-title", ["font: 700 17px/1.2 var(--font-display);"]);
  has('#discover-empty[data-pt-tone="error"] #discover-empty-text', ["color: var(--danger-text);"]);
  assert.match(glue, /const star = studioSlim\(\) && \(title === titles\.quiet \|\| title === titles\.error\);/);
  assert.match(glue, /discoverEmptyView\(\{ unavailable: true \}\)\.title/);
  assert.match(glue, /discoverEmptyView\(\{ error: "-" \}\)\.title/);
  assert.match(glue, /dataset\.state = "quiet"/);
  assert.match(glue, /attributeFilter: \["data-look", "data-fit"\]/);
});
