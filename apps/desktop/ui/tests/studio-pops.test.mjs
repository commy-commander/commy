import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { POPS_PICKS, popsPickIcon } = require("../scripts/studio-pops.js");

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const app = read("../scripts/app.js");
const glue = read("../scripts/studio-pops.js");
const css = read("../styles/studio-pops.css");
const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const selectors = [...css.replace(/@media[^{]+\{/g, "").matchAll(/(^|\})\s*([^{}@]+)\{/g)]
  .flatMap(match => match[2].split(/,\n/).map(part => part.trim()).filter(Boolean));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("the device menus and the DM place load with the new look, roomy only, from the tokens", () => {
  const sheet = html.indexOf('<link rel="stylesheet" id="studio-pops" href="styles/studio-pops.css" media="not all" />');
  assert.ok(sheet > html.indexOf('id="studio-float"'));
  assert.ok(sheet < html.indexOf('id="studio-phone"'));
  assert.ok(selectors.length > 30);
  for (const selector of selectors) assert.ok(selector.startsWith(R), selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
  assert.equal(/\/\/|\/\*/.test(glue.replace(/"[^"]*"/g, "")), false);
});

test("the glue loads after the floating parts and starts once after studioFloat", () => {
  const script = html.indexOf('<script src="scripts/studio-pops.js"></script>');
  assert.ok(script > html.indexOf('<script src="scripts/studio-float.js"></script>'));
  assert.equal(app.split("  if (studioLook()) studioPops();\n").length, 2);
  assert.ok(app.indexOf("  if (studioLook()) studioPops();") > app.indexOf("  if (studioLook()) studioFloat();"));
});

test("right-click device menus are the voice popover's glass slab with capsule picks and icons", () => {
  assert.deepEqual(POPS_PICKS, [["ap-device", "audio-pop"], ["vpop-device", "video-pop"]]);
  assert.equal(popsPickIcon("audio-pop", "input"), "mic");
  assert.equal(popsPickIcon("audio-pop", "output"), "headset");
  assert.equal(popsPickIcon("video-pop", undefined), "camera");
  assert.ok(html.includes('<div class="audio-pop hidden" id="audio-pop">'));
  assert.ok(html.includes('<div class="audio-pop video-pop hidden" id="video-pop"'));
  assert.ok(glue.includes('box.setAttribute("data-pops", "")'));
  assert.ok(glue.includes("else popsUnwrap(select);"));
  assert.match(rule(`${R} .audio-pop`), /width: 264px;/);
  assert.match(rule(`${R} .audio-pop`), /padding: 10px;/);
  assert.match(rule(`${R} .audio-pop`), /border-radius: var\(--radius-lg\);/);
  assert.match(rule(`${R} .audio-pop`), /var\(--panel-thin\);/);
  assert.match(rule(`${R} .audio-pop`), /backdrop-filter: blur\(var\(--fx-blur\)\)/);
  assert.match(rule(`${R} .audio-pop > .ap-title`), /font: 600 12px\/1 var\(--font\);/);
  assert.match(rule(`${R} .audio-pop > .ap-title`), /text-transform: none;/);
  assert.match(rule(`${R} .audio-pop > .ap-title`), /letter-spacing: 0;/);
  assert.match(rule(`${R} .audio-pop .pops-pick > select`), /height: 36px;/);
  assert.match(rule(`${R} .audio-pop .pops-pick > select`), /border-radius: var\(--radius-pill\);/);
  assert.match(rule(`${R} .audio-pop .pops-pick > select`), /font: 400 15px\/1 var\(--font\);/);
  assert.match(rule(`${R} .audio-pop > .ap-row > input.switch`), /width: 48px;\n  height: 28px;/);
  assert.match(rule(`${R} .audio-pop > .segmented > button`), /height: 34px;/);
  assert.match(rule(`${R} .audio-pop > .pp-link`), /color: var\(--link\);/);
  assert.match(rule(`${R} .audio-pop > .pp-link`), /font: 600 13\.5px\/1 var\(--font\);/);
});

test("the DMs key goes to the DM place in the new look and keeps the dropdown in the classic look", () => {
  assert.ok(app.includes('  $("home-btn").onclick = () => (wideRail() && !studioDesk() ? toggleDmMenu() : enterDmMode());\n'));
  assert.match(rule(`${R} .channel.dm-thread`), /min-height: 44px;\n  gap: 11px;\n  padding: 0 10px 0 8px;/);
  assert.match(rule(`${R} .channel.dm-thread.active:hover`), /min-height: 48px;/);
  assert.match(rule(`${R} .channel.dm-thread > .avatar`), /width: 30px; height: 30px;/);
  assert.match(rule(`${R} .channel.dm-thread .dm-name`), /font: 500 14px\/1 var\(--font\);/);
  assert.match(rule(`${R} .channel.dm-thread .dm-preview`), /font: 400 12px\/1\.1 var\(--font\);/);
  assert.match(rule(`${R} .channel.dm-thread .dm-time`), /12px/);
  assert.match(rule(`${R} .channel.dm-thread > .avatar > .presence-dot`), /width: 10px;/);
});
