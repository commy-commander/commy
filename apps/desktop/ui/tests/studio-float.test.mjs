import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { STAGE_KEYS, stageLiveWord } = require("../scripts/studio-float.js");

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio-float.css");
const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const selectors = [...css.replace(/@keyframes[^{]+\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, "").replace(/@media[^{]+\{/g, "").matchAll(/(^|\})\s*([^{}@]+)\{/g)]
  .flatMap(match => match[2].split(/,\n/).map(part => part.trim()).filter(Boolean));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("floating parts load with the new look, roomy only, from the tokens", () => {
  const sheet = html.indexOf('<link rel="stylesheet" id="studio-float" href="styles/studio-float.css" media="not all" />');
  assert.ok(sheet > html.indexOf('id="studio-dialogs"'));
  assert.ok(sheet < html.indexOf('id="studio-phone"'));
  assert.ok(selectors.length > 20);
  for (const selector of selectors) assert.ok(selector.startsWith(R), selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
});

test("menus are a nearer pane with capsule rows and a tick on the chosen row", () => {
  for (const id of ["server-menu", "channel-menu", "folder-menu", "member-menu", "msg-menu", "invite-menu", "composer-menu", "avatar-menu", "remind-pop"]) {
    assert.match(html, new RegExp(`class="pop-menu[^"]*"[^>]* id="${id}"|id="${id}"[^>]* class="pop-menu`), id);
  }
  assert.match(rule(`${R} .pop-menu`), /border-radius: 22px;/);
  assert.match(rule(`${R} .pop-menu`), /padding: 7px;/);
  assert.match(rule(`${R} .pop-menu button`), /min-height: 36px;/);
  assert.match(rule(`${R} .pop-menu button`), /border-radius: 15px;/);
  assert.match(rule(`${R} .pop-menu > .ctx-label`), /text-transform: none;/);
  assert.match(rule(`${R} .pop-menu > .ctx-sep`), /background: var\(--line\);/);
  assert.match(rule(`${R} .pop-menu button:hover:is(.ctx-radio, .ctx-check)::after`), /background: var\(--accent-lit\);/);
  assert.match(rule(`${R} .pop-menu button:hover:is(.ctx-radio, .ctx-check)::after`), /opacity: 0;/);
  assert.match(rule(`${R} .pop-menu button:is(.ctx-radio, .ctx-check).checked::after`), /opacity: 1;/);
  assert.match(rule(`${R} .pop-menu button.danger:hover`), /background: var\(--bg-hover\);/);
});

test("toasts are a capsule that rises once, and stay put for less motion", () => {
  assert.match(rule(`${R} .toast`), /min-height: 48px;/);
  assert.match(rule(`${R} .toast`), /border-radius: var\(--radius-pill\);/);
  assert.match(rule(`${R} .toast`), /font: 600 14\.5px\/1\.2 var\(--font\);/);
  assert.ok(css.includes("@keyframes float-toast-in { from { opacity: 0; translate: 0 80px; } }"));
  assert.ok(css.includes(`@media (prefers-reduced-motion: reduce) {\n  ${R} .pop-menu,\n  ${R} .toast { animation: none; }\n}`));
});

test("a stream watched from the chat is the guide's stream card with its keys across the bottom edge", () => {
  const C = `${R} .stage:not(.vr-grid .stage)`;
  const glue = read("../scripts/studio-float.js");
  const app = read("../scripts/app.js");
  for (const id of STAGE_KEYS) assert.ok(html.includes(`id="${id}"`), id);
  assert.ok(html.indexOf('<script src="scripts/studio-float.js"></script>') > html.indexOf('<script src="scripts/studio-dialogs.js"></script>'));
  assert.ok(app.includes("  if (studioLook()) studioDialogs();\n  if (studioLook()) studioFloat();\n"));
  assert.equal(stageLiveWord(true), "Live");
  assert.equal(stageLiveWord(false), "LIVE");
  assert.ok(glue.includes('studioKeyDress(key, "key sm sq", roomy)'));
  assert.ok(glue.includes('close.classList.toggle("danger", !roomy)'));
  assert.match(rule(`${C} > .stage-view`), /border-radius: 24px;/);
  assert.match(rule(`${C} > .stage-view > .stage-controls`), /bottom: -17px;/);
  assert.match(rule(`${C} > .stage-view > .stage-controls`), /opacity: 1;/);
  assert.match(rule(`${C} > .stage-view .stage-who > .live-badge`), /background: var\(--red\);/);
  assert.match(rule(`${C} > .stage-view .stage-chip`), /background: var\(--video-scrim\);/);
  assert.match(rule(`${C} > .stage-view .stage-ink-tools > .stage-btn.key.active`), /outline: var\(--ring-on\);/);
  assert.match(rule(`${C} > .stage-view .stage-ink-tools > .stage-btn.key.active`), /color: var\(--accent-lit\);/);
});
