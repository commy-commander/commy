import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { windowFit } = require("../scripts/look.js");
const { STUDIO_COMPACT_MARK, STUDIO_COMPACT_ROWS, STUDIO_COMPACT_MIXED_LABEL, studioCompactFace, studioCompactRowStates, studioCompactDot, studioCompactEmpty, studioCompactMenuLeft } = require("../scripts/studio-compact.js");

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio-compact.css");
const C = 'html[data-look="studio"][data-fit="compact"]';
const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const SLIM = 'html[data-look="studio"][data-fit="slim"]';
const glue = read("../scripts/studio-compact.js");
const app = read("../scripts/app.js");
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
  for (const selector of selectors) assert.ok(selector.startsWith(C) || selector.startsWith(R) || selector.startsWith(SLIM), selector);
  assert.deepEqual(selectors.filter(s => s.startsWith(SLIM)), [`${SLIM} .header-actions > #studio-more-btn`, `${SLIM} .composer-menu > #studio-cm-style`, `${SLIM} .studio-more-menu`]);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
  const script = '  <script src="scripts/studio-forum.js"></script>\n  <script src="scripts/studio-compact.js"></script>\n';
  assert.ok(html.includes(script));
  assert.equal(html.split("scripts/studio-compact.js").length, 2);
  const line = "  if (studioLook()) studioCompact();\n";
  assert.equal(app.split(line).length, 2);
  assert.ok(app.indexOf(line) > app.indexOf("  if (studioLook()) studioFloat();\n"));
  assert.doesNotMatch(glue, /\/\/|\/\*/);
});

test("the compact sheet uses the guide's own width breakpoints", () => {
  assert.deepEqual([...css.matchAll(/@media ([^{]+) \{/g)].map(m => m[1]), ["(min-width: 1180px)", "(max-width: 1179px)"]);
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

test("Pinned and Saved tuck under the Threads key at every desktop width, next to the bell", () => {
  assert.ok(css.includes(`${R} .header-actions > #pins-btn,\n${R} .header-actions > #threads-btn,\n${R} .header-actions > #saved-btn,\n${R} .header-actions > #studio-more-btn[data-empty],\n${R} .composer-menu > #studio-cm-style { display: none; }`));
  assert.ok(html.indexOf('id="pins-btn"') < html.indexOf('id="threads-btn"'));
  assert.ok(html.indexOf('id="threads-btn"') < html.indexOf('id="saved-btn"'));
  assert.ok(html.indexOf('id="saved-btn"') < html.indexOf('id="notify-btn"'));
  assert.ok(html.indexOf('id="notify-btn"') < html.indexOf('id="members-btn"'));
  assert.match(glue, /actions\.insertBefore\(more, saved && saved\.parentNode === actions \? saved\.nextSibling : null\);/);
});

test("under 1180 Text Style is the last row of the plus menu and opens the same popover above the field", () => {
  assert.ok(block("(max-width: 1179px)").includes(`${C} .composer-menu > #studio-cm-style { display: flex; }`));
  assert.match(glue, /item\.id = "studio-cm-style";/);
  assert.match(glue, /studioCompactIcon\("a-large-small", 18\)/);
  assert.match(glue, /document\.createTextNode\("Text Style"\)/);
  assert.match(glue, /plus\.appendChild\(item\);/);
  assert.match(glue, /getElementById\("text-style-btn"\)[\s\S]*button\.click\(\);[\s\S]*placePopup\(pop, field, "above"\);/);
});

test("the Threads key opens a menu of Threads, Pinned Messages and Saved Messages", () => {
  assert.match(glue, /more\.className = "icon-btn studio-more-btn";/);
  assert.match(glue, /studioCompactIcon\("chevronDown", 12\)/);
  assert.match(glue, /menu\.className = "pop-menu ctx-menu studio-more-menu hidden";/);
  assert.deepEqual(STUDIO_COMPACT_ROWS.map(row => [row.label, row.button, row.icon, row.open]), [["Threads", "threads-btn", "thread", "toggleThreadsPanel"], ["Pinned Messages", "pins-btn", "pin", "togglePinsPanel"], ["Saved Messages", "saved-btn", "bookmark", "toggleSavedView"]]);
  assert.match(app, /\$\("pins-btn"\)\.onclick = togglePinsPanel;/);
  assert.match(app, /\$\("threads-btn"\)\.onclick = toggleThreadsPanel;/);
  assert.match(app, /\$\("saved-btn"\)\.onclick = toggleSavedView;/);
  assert.match(rule(`${R} .header-actions > #studio-more-btn[data-dot]::after`), /background: var\(--accent-2\);/);
  assert.match(rule(`${R} .header-actions > #studio-more-btn:not([data-single])`), /width: auto;/);
});

test("the key wears Threads when it can, and opens the only row straight away when there is one", () => {
  const states = map => STUDIO_COMPACT_ROWS.map(row => ({ key: row.key, shown: Boolean(map[row.key]), badge: "" }));
  assert.deepEqual(studioCompactFace(STUDIO_COMPACT_ROWS, states({ threads: 1, pins: 1, saved: 1 })), { icon: "thread", label: "Threads", single: null });
  assert.deepEqual(studioCompactFace(STUDIO_COMPACT_ROWS, states({ pins: 1, saved: 1 })), { icon: "pin", label: STUDIO_COMPACT_MIXED_LABEL, single: null });
  assert.deepEqual(studioCompactFace(STUDIO_COMPACT_ROWS, states({ saved: 1 })), { icon: "bookmark", label: "Saved Messages", single: "saved" });
  assert.equal(studioCompactFace(STUDIO_COMPACT_ROWS, states({})), null);
});

test("a row follows its key, carries the badge, and lights the More dot", () => {
  const read = map => row => map[row.key];
  const both = studioCompactRowStates(STUDIO_COMPACT_ROWS, read({ pins: { shown: true, badge: "1" }, threads: { shown: true }, saved: { shown: true, badge: " 3 " } }));
  assert.deepEqual(both, [{ key: "threads", shown: true, badge: "" }, { key: "pins", shown: true, badge: "1" }, { key: "saved", shown: true, badge: "3" }]);
  assert.equal(studioCompactDot(both), true);
  assert.equal(studioCompactEmpty(both), false);
  const quiet = studioCompactRowStates(STUDIO_COMPACT_ROWS, read({ pins: { shown: false, badge: "1" }, threads: { shown: false, badge: "2" }, saved: { shown: true } }));
  assert.equal(studioCompactDot(quiet), false);
  const none = studioCompactRowStates(STUDIO_COMPACT_ROWS, read({ pins: {}, threads: {}, saved: {} }));
  assert.equal(studioCompactEmpty(none), true);
  assert.equal(studioCompactMenuLeft({ right: 926 }, 220, 1000), 706);
  assert.equal(studioCompactMenuLeft({ right: 100 }, 220, 1000), 8);
  assert.equal(studioCompactMenuLeft({ right: 1000 }, 220, 1000), 772);
});

test("the glue marks what it adds and only takes it away again in slim", () => {
  assert.equal(STUDIO_COMPACT_MARK, "data-studio-compact");
  assert.equal(glue.split("setAttribute(STUDIO_COMPACT_MARK, \"\")").length, 4);
  assert.match(glue, /if \(document\.documentElement\.dataset\.fit === "slim"\) studioCompactUndo\(\);\n  else studioCompactAdd\(\);/);
  assert.match(glue, /for \(const node of document\.querySelectorAll\(`\[\$\{STUDIO_COMPACT_MARK\}\]`\)\) node\.remove\(\);/);
  assert.match(glue, /if \(studioCompactStarted\) return;/);
});
