import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { titlebarTint, titlebarIdle } = require("../scripts/studio-titlebar.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const app = read("../scripts/app.js");
const glue = read("../scripts/studio-titlebar.js");
const css = read("../styles/studio-titlebar.css");
const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const selectors = [...css.replace(/@keyframes[^{]+\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, "").replace(/@media[^{]+\{/g, "").matchAll(/(^|\})\s*([^{}@]+)\{/g)]
  .flatMap(match => match[2].split(/,\n/).map(part => part.trim()).filter(Boolean));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("the title bar loads with the new look, roomy only, from the tokens", () => {
  const lines = html.split("\n");
  const parts = lines.findIndex(line => line.includes('id="studio-parts"'));
  assert.equal(lines[parts + 1], '  <link rel="stylesheet" id="studio-titlebar" href="styles/studio-titlebar.css" media="not all" />');
  const look = lines.indexOf('  <script src="scripts/profile-look.js"></script>');
  assert.equal(lines[look + 1], '  <script src="scripts/studio-titlebar.js"></script>');
  assert.ok(html.indexOf("scripts/studio-titlebar.js") < html.indexOf('src="scripts/app.js"'));
  assert.equal(app.split("  if (studioLook()) studioTitlebar();\n").length, 2);
  assert.ok(app.indexOf("studioTitlebar();") > app.indexOf("  if (studioLook()) studioFloat();"));
  assert.ok(selectors.length > 10);
  for (const selector of selectors) assert.ok(selector.startsWith(R), selector);
  for (const text of [css, glue]) {
    assert.equal(text.includes("/*"), false);
    assert.doesNotMatch(text, /(^|\s)\/\/|^\s*#/m);
    assert.deepEqual(text.match(/#[0-9a-fA-F]{3,8}\b/g), null);
    assert.doesNotMatch(text, /rgba?\(\s*\d/);
  }
});

test("the bar is 36 tall with no strip, and the app sits under it without overlap", () => {
  assert.match(rule(`${R} .titlebar`), /height: 36px;/);
  assert.match(rule(`${R} .titlebar`), /background: none;/);
  assert.match(rule(`${R} body.has-titlebar .app`), /height: calc\(100vh - 36px\);/);
  assert.match(rule(`${R} body.has-titlebar .app`), /margin-top: 36px;/);
  assert.match(rule(`${R} body.has-titlebar .app`), /padding-top: 6px;/);
  assert.match(rule(`${R} body.has-titlebar .offline-chip`), /top: 46px;/);
});

test("the server tile and name sit on the left as drawn", () => {
  assert.match(rule(`${R} .titlebar-drag`), /gap: 9px;/);
  assert.match(rule(`${R} .titlebar-drag`), /padding-left: 12px;/);
  const tile = rule(`${R} #titlebar-icon`);
  assert.match(tile, /width: 22px;/);
  assert.match(tile, /height: 22px;/);
  assert.match(tile, /border-radius: 6px;/);
  assert.match(tile, /background: var\(--server-tint, var\(--accent\)\);/);
  assert.match(tile, /color: var\(--white\);/);
  assert.match(tile, /font: 700 12px\/1 var\(--font\);/);
  assert.match(rule(`${R} .titlebar-name`), /font: 600 13px\/1 var\(--font\);/);
  assert.match(rule(`${R} .titlebar-name`), /color: var\(--text-muted\);/);
});

test("three caption keys 46 by 36, Close goes danger red on hover, faint when not in front", () => {
  assert.match(rule(`${R} .titlebar-btn`), /width: 46px;/);
  assert.match(rule(`${R} .titlebar-btn`), /height: 36px;/);
  assert.match(rule(`${R} .titlebar-btn`), /color: var\(--text-muted\);/);
  assert.match(rule(`${R} .titlebar-btn svg`), /width: 10px;/);
  assert.match(rule(`${R} .titlebar .titlebar-btn:hover`), /background: var\(--bg-hover\);/);
  assert.match(rule(`${R} .titlebar .titlebar-btn:hover`), /color: var\(--header-primary\);/);
  assert.match(rule(`${R} .titlebar .titlebar-btn.titlebar-close:hover`), /background: var\(--danger\);/);
  assert.match(rule(`${R} .titlebar .titlebar-btn.titlebar-close:hover`), /color: var\(--danger-ink\);/);
  assert.match(rule(`${R} .titlebar.is-idle :is(.titlebar-name, .titlebar-btn)`), /color: var\(--text-faint\);/);
  assert.match(rule(`${R} .titlebar.is-idle #titlebar-icon`), /opacity: \.55;/);
});

test("the tile takes the server's colour and the bar knows when the window is in front", () => {
  const tint = id => `tint-${id}`;
  assert.equal(titlebarTint({ id: "rh" }, tint), "tint-rh");
  assert.equal(titlebarTint(null, tint), "");
  assert.equal(titlebarTint({ id: "rh" }, null), "");
  assert.equal(titlebarIdle(true, "visible"), false);
  assert.equal(titlebarIdle(false, "visible"), true);
  assert.equal(titlebarIdle(true, "hidden"), true);
  assert.match(glue, /onFocusChanged/);
  assert.match(glue, /bar\.dataset\.studioBar\) return;/);
});
