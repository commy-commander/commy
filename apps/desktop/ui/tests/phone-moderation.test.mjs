import { englishSource } from "./i18n-global.mjs";
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-moderation.css");
const app = englishSource(read("scripts", "app.js"));
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';
const P = `${SLIM} .settings-layer .settings-content`;

const topLevel = head => head.split(/,(?![^(]*\))/);
const selectors = css.split("}").map(block => block.split("{")[0]).flatMap(topLevel).map(s => s.trim()).filter(Boolean);
const rule = sel => {
  const at = css.indexOf(`${sel} {`);
  assert.notEqual(at, -1, `missing ${sel}`);
  return css.slice(at, css.indexOf("}", at));
};

test("every moderation rule is for the phone layout of the new look, with tokens and no comments", () => {
  assert.ok(selectors.length > 30);
  for (const selector of selectors) assert.ok(selector.startsWith(SLIM), selector);
  assert.equal(/#[0-9a-fA-F]{3,8}\b/.test(css), false);
  assert.equal(css.includes("/*"), false);
});

test("the moderation sheet loads right after the settings page sheet", () => {
  assert.match(html, /id="studio-phone-settings-page" href="styles\/phone-settings-page\.css" media="not all" \/>\n\s*<link rel="stylesheet" id="studio-phone-moderation" href="styles\/phone-moderation\.css" media="not all" \/>/);
});

test("an audit row wraps its sentence and puts the time on its own line", () => {
  assert.match(rule(`${P} .audit-main`), /flex-direction: column;/);
  const text = rule(`${P} .audit-text`);
  assert.match(text, /white-space: normal;/);
  assert.match(text, /overflow: visible;/);
  assert.doesNotMatch(text, /text-overflow/);
  assert.match(rule(`${P} .audit-filters`), /grid-template-columns: minmax\(0, 1fr\);/);
});

test("an audit row is a button that opens its details", () => {
  assert.match(app, /const head = el\("button", "audit-head"\);/);
  assert.match(app, /head\.setAttribute\("aria-expanded", String\(open\)\);/);
  assert.match(app, /auditView\.open = open \? null : key;/);
  assert.match(app, /for \(const \[term, value\] of auditFacts\(entry\)\)/);
});

test("ban, kick and report keys sit on their own full-width row at touch size", () => {
  assert.match(rule(`${P} .removed-actions`), /flex: 1 1 100%;/);
  assert.match(rule(`${P} .report-actions`), /grid-template-columns: repeat\(2, minmax\(0, 1fr\)\);/);
  assert.match(rule(`${P} .report-actions > .btn-link`), /height: 44px;/);
  assert.match(rule(`${P} .manage-row .row-more`), /width: 44px;\s*height: 44px;/);
  assert.match(rule(`${P} .automod-roles > label`), /min-height: 44px;/);
  assert.match(app, /"btn-secondary btn-small report-jump", "Jump to Message"/);
});

test("the report reason is the one flat chip, never red", () => {
  const chip = rule(`${P} .report-reason`);
  assert.match(chip, /background: var\(--bg-active\);/);
  assert.match(chip, /color: var\(--header-primary\);/);
});

test("a held message keeps its author's whole name and wraps the mark under it", () => {
  assert.match(rule(`${SLIM} .app[data-pane] .msg.held .msg-head`), /flex-wrap: wrap;\s*height: auto;/);
  assert.match(rule(`${SLIM} .app[data-pane] .msg.held .msg-author`), /flex-shrink: 0;/);
});
