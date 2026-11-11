import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { STUDIO_SAVED_CLOSE_KEY, studioSavedEntry, studioSavedInk } = require("../scripts/studio-saved.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const app = read("../scripts/app.js");
const css = read("../styles/studio-saved.css");
const glue = read("../scripts/studio-saved.js");
const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const selectors = [...css.replace(/@keyframes[^{]+\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, "").replace(/@media[^{]+\{/g, "").matchAll(/(^|\})\s*([^{}@]+)\{/g)]
  .flatMap(match => match[2].split(/,\n/).map(part => part.trim()).filter(Boolean));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("Saved loads with the new look, roomy only, from the tokens", () => {
  assert.ok(html.includes('  <link rel="stylesheet" id="studio-settings" href="styles/studio-settings.css" media="not all" />\n  <link rel="stylesheet" id="studio-saved" href="styles/studio-saved.css" media="not all" />\n'));
  assert.ok(html.includes('  <script src="scripts/studio-dock.js"></script>\n  <script src="scripts/studio-saved.js"></script>\n'));
  assert.ok(html.indexOf('src="scripts/studio-saved.js"') < html.indexOf('src="scripts/app.js"'));
  assert.equal(app.split("  if (studioLook()) studioSaved();\n").length, 2);
  assert.ok(app.indexOf("studioSaved();") > app.indexOf("  if (studioLook()) studioFloat();"));
  assert.ok(selectors.length > 20);
  for (const selector of selectors) assert.ok(selector.startsWith(R), selector);
  for (const source of [css, glue]) {
    assert.equal(source.includes("/*"), false);
    assert.deepEqual(source.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  }
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
  assert.doesNotMatch(glue, /^\s*\/\//m);
});

test("Saved takes the chat's place with the guide's head and sections", () => {
  assert.match(rule(`${R} .main:has(> .saved-view:not(.hidden)) > :not(.chat-header):not(.saved-view)`), /visibility: hidden;/);
  assert.match(rule(`${R} .saved-view`), /padding: 16px 12px 12px;/);
  assert.match(rule(`${R} .saved-view`), /background: none;/);
  assert.match(rule(`${R} .saved-head`), /padding: 0 6px 14px;/);
  assert.match(rule(`${R} .saved-head`), /border-bottom: 1px solid var\(--line\);/);
  assert.match(rule(`${R} .saved-title`), /font: 700 20px\/1 var\(--font-display\);/);
  assert.match(rule(`${R} .saved-sub`), /color: var\(--text-muted\);/);
  assert.match(rule(`${R} .saved-head > .key.sq`), /width: 36px;/);
  assert.equal(STUDIO_SAVED_CLOSE_KEY, "key sq");
  assert.match(rule(`${R} .saved-section`), /margin: 16px 8px 6px;/);
  assert.match(rule(`${R} .saved-section`), /text-transform: none;/);
  assert.match(rule(`${R} .saved-section > span:first-child`), /font: 600 13px\/1 var\(--font\);/);
  assert.match(rule(`${R} .saved-section > span:last-child`), /background: var\(--bg-active\);/);
  assert.match(rule(`${R} .saved-section > span:last-child`), /font: 700 12px\/20px var\(--font\);/);
});

test("rows are capsule-cornered with a 36 picture, two clamped lines and a hover x", () => {
  assert.match(rule(`${R} .saved-row`), /grid-template-columns: 36px minmax\(0, 1fr\) 32px;/);
  assert.match(rule(`${R} .saved-row`), /padding: 10px;/);
  assert.match(rule(`${R} .saved-row`), /border-radius: 14px;/);
  assert.match(rule(`${R} .saved-row > .avatar`), /width: 36px;/);
  assert.match(rule(`${R} .saved-author`), /color: var\(--saved-ink, var\(--header-primary\)\);/);
  assert.match(rule(`${R} .saved-origin`), /font: 500 12\.5px\/1\.2 var\(--font\);/);
  assert.match(rule(`${R} .saved-time`), /color: var\(--text-faint\);/);
  assert.match(rule(`${R} .saved-text`), /-webkit-line-clamp: 2;/);
  assert.match(rule(`${R} .saved-text.muted`), /font-style: italic;/);
  assert.match(rule(`${R} .saved-remove`), /width: 32px;/);
  assert.match(rule(`${R} .saved-remove`), /opacity: 0;/);
  assert.match(rule(`${R} .saved-remove svg`), /width: 19px !important;/);
});

test("the reminder chip is quiet glass, then tungsten once due", () => {
  assert.match(rule(`${R} .saved-chip`), /height: 24px;/);
  assert.match(rule(`${R} .saved-chip`), /padding: 0 10px 0 8px;/);
  assert.match(rule(`${R} .saved-chip`), /background: rgb\(var\(--lamp\) \/ \.05\);/);
  assert.match(rule(`${R} .saved-chip`), /box-shadow: inset 0 0 0 1px var\(--line\);/);
  assert.match(rule(`${R} .saved-chip svg`), /width: 13px !important;/);
  assert.match(rule(`${R} .saved-chip.due`), /background: var\(--accent-2\);/);
  assert.match(rule(`${R} .saved-chip.due`), /color: var\(--accent-2-ink\);/);
});

test("nothing saved is the quiet 64 well with the app's words", () => {
  assert.match(rule(`${R} .saved-empty`), /padding: 34px 18px 26px;/);
  assert.match(rule(`${R} .saved-empty-mark`), /width: 64px;/);
  assert.match(rule(`${R} .saved-empty-mark`), /box-shadow: var\(--slot\);/);
  assert.match(rule(`${R} .saved-empty-mark svg`), /width: 28px !important;/);
  assert.match(rule(`${R} .saved-empty strong`), /font: 700 17px\/1\.2 var\(--font-display\);/);
  assert.match(rule(`${R} .saved-empty > span`), /max-width: 340px;/);
});

test("a saved row finds its entry and wears the author's role colour from the open server", () => {
  const entry = { message: "x1", server: "s1", author_id: "m1" };
  const reminded = { message: "x2", server: "s1", author_id: "m2" };
  const store = { saved: [entry], reminders: [{ id: "r", due: 1, entry: reminded }] };
  assert.equal(studioSavedEntry("x1", store), entry);
  assert.equal(studioSavedEntry("x2", store), reminded);
  assert.equal(studioSavedEntry("nope", store), null);
  assert.equal(studioSavedEntry("x1", null), null);
  const server = { id: "s1", members: [{ id: "m1", color: "9b7bff" }, { id: "m2", color: null }] };
  const ink = (color, studio) => `ink:${color}:${studio}`;
  assert.equal(studioSavedInk(entry, server, ink), "ink:9b7bff:true");
  assert.equal(studioSavedInk(reminded, server, ink), "");
  assert.equal(studioSavedInk({ ...entry, server: "s2" }, server, ink), "");
  assert.equal(studioSavedInk({ message: "x3", dm: "m1", server: null, author_id: "m1" }, server, ink), "");
  assert.equal(studioSavedInk(entry, null, ink), "");
});
