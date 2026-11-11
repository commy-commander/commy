import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-viewer.css");
const glue = read("scripts", "phone-viewer.js");
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';
const rule = sel => {
  const at = css.indexOf(`${sel} {`);
  assert.ok(at >= 0, `missing ${sel}`);
  return css.slice(at, css.indexOf("}", at));
};

test("every picture viewer rule is for the phone layout in the new look", () => {
  const selectors = css.replace(/\{[^}]*\}/g, "\n").split(/[\n,]/).map(s => s.trim()).filter(Boolean);
  assert.ok(selectors.length > 10);
  for (const sel of selectors) assert.ok(sel.startsWith(SLIM), sel);
});

test("the viewer sheet has no hex colours and no comments", () => {
  assert.doesNotMatch(css.replace(/linear-gradient\(#000 0 0\)/g, ""), /#[0-9a-fA-F]{3,8}\b/);
  assert.doesNotMatch(css, /\/\*/);
  assert.doesNotMatch(glue, /\/\/|\/\*/);
});

test("the viewer sheet loads right after the phone sheet and its script after the phone script", () => {
  assert.match(html, /<link rel="stylesheet" id="studio-phone" href="styles\/phone-touch\.css" media="not all" \/>\n  <link rel="stylesheet" id="studio-phone-viewer" href="styles\/phone-viewer\.css" media="not all" \/>/);
  assert.match(html, /<script src="scripts\/studio-phone\.js"><\/script>\n  <script src="scripts\/phone-viewer\.js"><\/script>/);
});

test("the app dims and blurs behind the picture", () => {
  const box = rule(`${SLIM} #lightbox`);
  assert.match(box, /background: var\(--scrim\)/);
  assert.match(box, /backdrop-filter: blur\(16px\)/);
});

test("the picture fills the width with 14 margins and a 16 radius", () => {
  const pic = rule(`${SLIM} #lightbox-img`);
  assert.match(pic, /calc\(100vw - 28px\)/);
  assert.match(pic, /border-radius: 16px/);
  assert.match(pic, /translate: -50% -60%/);
});

test("one glass tray sits near the bottom with the source and 44 tall keys", () => {
  const tray = rule(`${SLIM} #lightbox .lightbox-actions`);
  assert.match(tray, /var\(--panel-thin\)/);
  assert.match(tray, /padding: 6px 6px 6px 16px/);
  assert.match(tray, /bottom: calc\(38px \+ env\(safe-area-inset-bottom\)\)/);
  const key = rule(`${SLIM} #lightbox .lightbox-action`);
  assert.match(key, /height: 44px/);
  assert.match(key, /padding: 0 16px 0 13px/);
  assert.match(key, /font: 600 14\.5px\/1 var\(--font\)/);
  const src = rule(`${SLIM} #lightbox-source.linked,\n${SLIM} #lightbox-source.linked:hover`);
  assert.match(src, /color: var\(--link\)/);
});

test("the script only moves the source into the tray in the phone layout and puts it back otherwise", () => {
  assert.match(glue, /const slim = studioSlim\(\);/);
  assert.match(glue, /tray\.prepend\(source\)/);
  assert.match(glue, /box\.insertBefore\(source, tray\)/);
  assert.doesNotMatch(glue, /lightbox-save[^]*hidden/);
});

test("on the phone the viewer holds one history step that Back closes and a tap or Esc steps back", () => {
  assert.match(glue, /return studioSlim\(\) && !\$\("lightbox"\)\.classList\.contains\("hidden"\);/);
  assert.match(glue, /layerStep\("ptViewer", open, phoneViewer\.step, history\.state\)/);
  assert.match(glue, /if \(step === "push"\) history\.pushState\(\{ \.\.\.\(history\.state \|\| \{ pane: phonePane\(\) \}\), ptViewer: true \}, ""\);/);
  assert.match(glue, /if \(step === "back"\) history\.back\(\);/);
  assert.match(glue, /if \(\(history\.state && history\.state\.ptViewer\) \|\| !viewerShown\(\)\) return;\n    phoneViewer\.step = false;\n    closeLightbox\(\);/);
});
