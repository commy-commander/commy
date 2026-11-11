import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-settings-page.css");
const js = read("scripts", "phone-settings-page.js");
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';

const selectors = text => text
  .replace(/\{[^{}]*\}/g, "\n")
  .split(/,\s*\n|\n/)
  .map(s => s.trim())
  .filter(Boolean);
const rule = (sel, prop) => {
  const at = css.indexOf(`${sel} {`);
  assert.notEqual(at, -1, `missing ${sel}`);
  const body = css.slice(at, css.indexOf("}", at));
  const hit = body.match(new RegExp(`(?:^|[\\s;{])${prop}: ([^;]+);`));
  assert.ok(hit, `${sel} has no ${prop}`);
  return hit[1].trim();
};
const P = `${SLIM} .settings-layer .settings-content`;

test("every settings page rule is for the phone layout of the new look", () => {
  const list = selectors(css);
  assert.ok(list.length > 100);
  for (const sel of list) assert.ok(sel.startsWith(SLIM), sel);
});

test("the settings page parts carry no hex colours and no comments", () => {
  for (const text of [css, js]) {
    assert.doesNotMatch(text, /#[0-9a-fA-F]{3,8}\b/);
    assert.doesNotMatch(text, /\/\*/);
    assert.doesNotMatch(text, /(^|[^:])\/\/ /m);
  }
});

test("the settings page sheet loads right after the phone sheet and its script after the phone script", () => {
  assert.match(html, /id="studio-phone" href="styles\/phone-touch\.css" media="not all" \/>\n(\s*<link rel="stylesheet" id="studio-phone-[a-z-]+" href="styles\/phone-[a-z-]+\.css" media="not all" \/>\n)*\s*<link rel="stylesheet" id="studio-phone-settings-page" href="styles\/phone-settings-page\.css" media="not all" \/>/);
  assert.match(html, /<script src="scripts\/studio-phone\.js"><\/script>\n(\s*<script src="scripts\/phone-[a-z-]+\.js"><\/script>\n)*\s*<script src="scripts\/phone-settings-page\.js"><\/script>/);
});

test("the page sits in the slab with the guide's padding and twelve between cards", () => {
  assert.equal(rule(`${SLIM} .settings-layer .settings-content-inner`, "padding"), "14px 12px 60px");
  assert.equal(rule(`${P} .settings-page`, "gap"), "12px");
  assert.equal(rule(`${P} .settings-page > .set-lead`, "font"), "400 14.5px/1.45 var(--font)");
});

test("each card is a faint radius 20 card with a hairline, padding 16 and a Funnel 17 title", () => {
  const card = `${P} .set-lift`;
  assert.equal(rule(card, "border-radius"), "20px");
  assert.equal(rule(card, "padding"), "16px");
  assert.equal(rule(card, "background"), "rgb(var(--lamp) / .03)");
  assert.equal(rule(card, "box-shadow"), "inset 0 0 0 1px var(--line)");
  assert.equal(rule(`${P} :is(.set-pane, .set-lift) > .settings-subhead`, "font"), "700 17px/1.2 var(--font-display)");
});

test("fields are capsule slots 48 tall under 600 13 muted labels", () => {
  assert.equal(rule(`${P} .field-label`, "font"), "600 13px/13px var(--font)");
  const slot = `${P} :where(input[type="text"], input[type="password"], input[type="search"], input[type="number"], input[type="email"], input[type="url"], select, textarea)`;
  assert.equal(rule(slot, "height"), "48px");
  assert.equal(rule(slot, "border-radius"), "999px");
  assert.equal(rule(slot, "box-shadow"), "var(--slot)");
});

test("sliders are a 6 px slot track with a ceramic knob 26 and the value 600 14 on the right", () => {
  const track = `${P} input[type="range"]:not(#sensitivity-db)::-webkit-slider-runnable-track`;
  assert.equal(rule(track, "height"), "6px");
  assert.equal(rule(track, "box-shadow"), "var(--slot)");
  const knob = `${P} input[type="range"]:not(#sensitivity-db)::-webkit-slider-thumb`;
  assert.equal(rule(knob, "width"), "26px");
  assert.equal(rule(knob, "background"), "var(--cream)");
  assert.equal(rule(`${P} .slider-row > .setting-title > span[id$="-label"]`, "font"), "600 14px/1 var(--font)");
});

test("keys in a row are glass 44, Save is the one full width primary, the danger key is red and full width", () => {
  const row = `${P} :is(.setting-row, .account-line, .roles-head) > :is(.btn-primary, .btn-secondary)`;
  assert.equal(rule(row, "height"), "44px");
  assert.equal(rule(row, "background"), "var(--k-plain)");
  assert.match(css, /:is\(#rename-save[^{]*\) \{\n  flex: 1 0 100%;\n  order: 9;\n  width: 100%;\n  height: 48px;/);
  const danger = `${P} .danger-zone > .btn-danger`;
  assert.equal(rule(danger, "width"), "100%");
  assert.equal(rule(danger, "height"), "48px");
  assert.equal(rule(danger, "background"), "var(--pillow-top), var(--danger)");
});

test("two way choices are a two part slot with the picked half lit and ringed", () => {
  assert.equal(rule(`${P} .segmented:not(.voice-filters)`, "border-radius"), "22px");
  const on = `${P} .segmented:not(.voice-filters) > button:not(.key).active`;
  assert.equal(rule(on, "background"), "var(--bg-active)");
  assert.equal(rule(on, "outline"), "var(--ring-on)");
});

test("switches wear the phone toggle", () => {
  const sw = `${P} input.switch`;
  assert.equal(rule(sw, "width"), "54px");
  assert.equal(rule(sw, "height"), "32px");
  assert.equal(rule(sw, "background"), "var(--tog-off)");
  assert.equal(rule(`${P} input.switch:checked::after`, "translate"), "22px 0");
});

test("the phone page script only moves things in the phone layout and puts them back after", () => {
  assert.match(js, /if \(!studioSlim\(\)\) \{\n    phoneSetRestore\(\);/);
  assert.match(js, /parent\.insertBefore\(node, next\)/);
  assert.match(js, /studioSlim\(\) && icon && !icon\.disabled/);
});
