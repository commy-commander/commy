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
  const ANY_FIT = ['html[data-look="studio"] #titlebar-icon.is-logo', 'html[data-look="studio"] .titlebar-icon.server-icon', 'html[data-look="studio"][data-fit="slim"] body.has-titlebar'];
  for (const selector of selectors) assert.ok(selector.startsWith(R) || ANY_FIT.some(start => selector.startsWith(start)), selector);
  for (const text of [css, glue]) {
    assert.equal(text.includes("/*"), false);
    assert.doesNotMatch(text, /(^|\s)\/\/|^\s*#/m);
    assert.deepEqual(text.match(/#[0-9a-fA-F]{3,8}\b/g), null);
    assert.doesNotMatch(text, /rgba?\(\s*\d/);
  }
});

test("there is one top bar: the window's keys sit in the row of the servers and the profile, and the app fills the window", () => {
  assert.match(rule(`${R} .titlebar`), /left: auto; height: calc\(var\(--st-mt\) \* 2 \+ 50px\); background: none;/);
  assert.match(rule(`${R} .titlebar-drag`), /display: none;/);
  assert.match(rule(`${R} body.has-titlebar .app`), /height: 100vh; margin-top: 0;/);
  assert.match(rule(`${R} body.has-titlebar .rail`), /padding-right: calc\(146px - var\(--st-mr\)\);/);
  assert.match(rule(`${R} body.has-titlebar .offline-chip`), /top: calc\(var\(--st-mt\) \+ 9px\);/);
});

test("the window moves by the empty part of the top row, and a double press there makes it full size", () => {
  const start = app.indexOf("const WINDOW_DRAG_STRIP = 70;");
  const end = app.indexOf("\n}\n", app.indexOf("function windowDragSpot(")) + 3;
  const spot = new Function(`${app.slice(start, end)}; return windowDragSpot;`)();
  assert.equal(spot({ id: "rail" }, "roomy", 30), true);
  assert.equal(spot({ id: "rail" }, "compact", 30), true);
  assert.equal(spot({ id: "rail" }, "slim", 30), false);
  assert.equal(spot({ id: "app" }, "roomy", 4), true);
  assert.equal(spot({ id: "app" }, "roomy", 400), false);
  assert.equal(spot({ id: "home-btn" }, "roomy", 30), false);
  assert.ok(app.includes('$("app").addEventListener("mousedown", e => { if (e.button === 0 && e.detail === 1 && windowDragSpot(e.target, document.documentElement.dataset.fit, e.clientY)) act("drag"); });'));
  assert.ok(app.includes('$("app").addEventListener("dblclick", e => { if (windowDragSpot(e.target, document.documentElement.dataset.fit, e.clientY)) act("toggle-maximize"); });'));
});

test("three caption keys 46 wide and as tall as the row, lit only in their middle, Close goes danger red on hover, faint when not in front", () => {
  assert.match(rule(`${R} .titlebar-btn`), /width: 46px;/);
  assert.match(rule(`${R} .titlebar-btn`), /height: 100%; padding-block: calc\(var\(--st-mt\) \+ 7px\); background-clip: content-box;/);
  assert.match(rule(`${R} .titlebar-btn`), /color: var\(--text-muted\);/);
  assert.match(rule(`${R} .titlebar-btn svg`), /width: 10px;/);
  assert.match(rule(`${R} .titlebar .titlebar-btn:hover`), /background: var\(--bg-hover\);/);
  assert.match(rule(`${R} .titlebar .titlebar-btn:hover`), /color: var\(--header-primary\);/);
  assert.match(rule(`${R} .titlebar .titlebar-btn.titlebar-close:hover`), /background: var\(--danger\);/);
  assert.match(rule(`${R} .titlebar .titlebar-btn.titlebar-close:hover`), /color: var\(--danger-ink\);/);
  assert.match(rule(`${R} .titlebar.is-idle .titlebar-btn`), /color: var\(--text-faint\);/);
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

test("the tile is round like a face, and the star in its place keeps its own shape", () => {
  assert.ok(css.includes('html[data-look="studio"] .titlebar-icon.server-icon, html[data-look="studio"] .titlebar-icon.server-icon:hover { border-radius: 50%; }'));
  assert.ok(css.includes('html[data-look="studio"] #titlebar-icon.is-logo { background: none; box-shadow: none; border-radius: 0; }'));
  assert.match(app, /icon\.classList\.add\("is-logo"\);/);
  assert.match(app, /icon\.classList\.remove\("is-logo"\);/);
  assert.ok(app.includes('"images/commy-star-small.svg"'));
});

test("in a slim window the pages start under the title bar, so their top keys can be pressed", () => {
  assert.ok(css.includes('html[data-look="studio"][data-fit="slim"] body.has-titlebar { position: fixed; inset: 32px 0 0; width: auto; height: auto; min-height: 0; transform: translateZ(0); --app-height: calc(100dvh - 32px); }'));
  assert.ok(css.includes('html[data-look="studio"][data-fit="slim"] body.has-titlebar > .titlebar { top: -32px; }'));
  assert.ok(css.includes('html[data-look="studio"][data-fit="slim"] body.has-titlebar .app { height: 100%; margin-top: 0; }'));
});
