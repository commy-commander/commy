import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-menus.css");
const glue = read("scripts", "phone-menus.js");
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';
const flat = css.replace(/@media \(hover: hover\) \{\n([\s\S]*?)\n\}\n/g, "$1\n");

const rule = selector => {
  const at = flat.indexOf(`${selector} {`);
  assert.notEqual(at, -1, selector);
  return flat.slice(at, flat.indexOf("}", at));
};

test("every phone menu rule is for the phone app in the new look, with tokens and no comments", () => {
  const selectors = flat.split("}").map(block => block.split("{")[0]).flatMap(head => head.split(",")).map(s => s.trim()).filter(Boolean);
  assert.ok(selectors.length > 30);
  for (const selector of selectors) assert.ok(selector.startsWith(SLIM), selector);
  assert.equal(/#[0-9a-fA-F]{3,8}\b/.test(css.replace(/#(server|channel)-menu/g, "")), false);
  assert.equal(css.includes("/*"), false);
  assert.equal(/\/\/|\/\*/.test(glue), false);
  assert.equal(css.includes(":is("), false);
});

test("the phone menu files sit with the other phone files, and the off-key sheet stays last", () => {
  assert.match(html, /id="studio-phone" href="styles\/phone-touch\.css" media="not all" \/>\n(\s*<link rel="stylesheet" id="studio-phone-[a-z-]+" href="styles\/phone-[a-z-]+\.css" media="not all" \/>\n)*\s*<link rel="stylesheet" id="studio-phone-menus" href="styles\/phone-menus\.css" media="not all" \/>\n\s*<link rel="stylesheet" id="studio-phone-off" href="styles\/phone-off\.css" media="not all" \/>/);
  assert.match(html, /<script src="scripts\/studio-phone\.js"><\/script>\n(\s*<script src="scripts\/phone-[a-z-]+\.js"><\/script>\n)*\s*<script src="scripts\/phone-menus\.js"><\/script>/);
  const links = [...html.matchAll(/<link rel="stylesheet" id="(studio-phone[a-z-]*)"/g)].map(m => m[1]);
  assert.equal(links.at(-1), "studio-phone-off");
  assert.equal(links.at(-2), "studio-phone-menus");
});

test("the server and channel menus are the hold sheet: flush to the bottom, glass, a grab, rows of 56", () => {
  const sheet = rule(`${SLIM} #channel-menu:not(.hidden)`);
  assert.match(sheet, /left: 0 !important;\n  right: 0;\n  top: auto !important;\n  bottom: 0;/);
  assert.match(sheet, /padding: 30px 14px calc\(var\(--hz-safe\) \+ 8px\);/);
  assert.match(sheet, /border-radius: var\(--radius-sheet\) var\(--radius-sheet\) 0 0;/);
  assert.match(sheet, /var\(--sheet-bg\)/);
  assert.match(sheet, /backdrop-filter: blur\(var\(--fx-blur\)\) saturate\(var\(--fx-sat\)\);/);
  assert.match(sheet, /0 0 0 100vmax var\(--scrim\)/);
  const grab = rule(`${SLIM} #channel-menu::before`);
  assert.match(grab, /position: sticky;/);
  assert.match(grab, /width: 40px;\n  height: 5px;\n  margin: -20px 0 15px;\n  border-radius: 3px;\n  background: var\(--text-faint\);\n  opacity: \.6;/);
  const row = rule(`${SLIM} #channel-menu > .arow`);
  assert.match(row, /gap: 15px;/);
  assert.match(row, /min-height: 56px;\n  padding: 0 20px;/);
  assert.match(row, /border-radius: 999px;/);
  assert.match(row, /font: 500 16\.5px\/1 var\(--font\);\n  color: var\(--header-primary\);/);
  assert.match(rule(`${SLIM} #channel-menu > .ctx-title + .arow`), /margin-bottom: 5px; background: var\(--pillow-top\), var\(--bg-raised\); box-shadow: var\(--pillow-sm\);/);
  assert.match(rule(`${SLIM} #channel-menu > .arow > .i`), /width: 23px; height: 23px; color: var\(--text-muted\);/);
});

test("levels carry a check when picked and no radio marks; mute is the phone switch", () => {
  assert.match(rule(`${SLIM} #channel-menu > .arow::after`), /content: none;/);
  assert.match(rule(`${SLIM} #channel-menu > .arow > .tick`), /width: 22px; height: 22px; color: var\(--accent-lit\); opacity: 0;/);
  assert.match(rule(`${SLIM} #channel-menu > .arow[aria-checked="true"] > .tick`), /opacity: 1; scale: 1;/);
  assert.match(glue, /studioIconMarkup\("check", "tick"\)/);
  assert.match(glue, /el\("i", "tog"\)/);
  assert.match(glue, /if \(button\.classList\.contains\("ctx-check"\)\) return "switch";/);
  assert.match(glue, /const PHONE_MENU_SWITCH_WORDS = \{ "sm-mute": "Mute Server" \};/);
  assert.match(glue, /if \(slim\) dressPhoneMenuRow\(button\);\n    else undressPhoneMenuRow\(button\);/);
});

test("a row under the finger does not stay lit on touch: the classic hover is cancelled and the lamp waits for a real pointer", () => {
  assert.match(rule(`${SLIM} #channel-menu > .arow:focus`), /background: none; color: var\(--header-primary\); outline: none;/);
  assert.match(rule(`${SLIM} #channel-menu > .arow:active`), /background: var\(--bg-active\);/);
  const hover = css.slice(css.indexOf("@media (hover: hover) {"));
  assert.ok(hover.length > 20);
  assert.match(hover, /#channel-menu > \.arow:hover \{ background: rgb\(var\(--lamp\) \/ \.06\); \}/);
});
