import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { STAGE_KEYS, PICKER_KEYS, stageLiveWord, railServerId } = require("../scripts/studio-float.js");

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio-float.css");
const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const SLIM = 'html[data-look="studio"][data-fit="slim"]';
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
  for (const selector of selectors.filter(s => !s.startsWith(SLIM))) assert.ok(selector.startsWith(R), selector);
  assert.deepEqual(selectors.filter(s => s.startsWith(SLIM)), [`${SLIM} .search-empty .sstar`]);
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

test("the pickers are one 400 by 452 slab with slot tabs, a capsule search, a hairline rail and a slot foot", () => {
  const glue = read("../scripts/studio-float.js");
  assert.match(rule(`${R} :is(.emoji-picker, .emoji-pop, .gif-picker)`), /width: 400px;\n  height: 452px;\n  padding: 10px;/);
  assert.match(rule(`${R} :is(.emoji-picker, .emoji-pop) > .picker-tabs`), /padding: 4px;\n  border-radius: var\(--radius-pill\);\n  background: var\(--input-bg\);\n  box-shadow: var\(--slot\);/);
  assert.match(rule(`${R} :is(.emoji-picker, .emoji-pop) > .picker-tabs > button.active`), /outline: var\(--ring-on\);/);
  assert.match(rule(`${R} .emoji-browser .eb-search`), /height: 36px;\n  padding: 0 14px;\n  border-radius: var\(--radius-pill\);/);
  assert.match(rule(`${R} .emoji-browser .eb-tone`), /width: 36px;\n  height: 36px;\n  border-radius: 50%;\n  background: var\(--glass-key\);/);
  assert.match(rule(`${R} .emoji-browser .eb-rail`), /flex: 0 0 40px;[\s\S]*border-right: 1px solid var\(--line\);/);
  assert.match(rule(`${R} .emoji-browser .eb-rail-server`), /width: 30px;\n  height: 30px;\n  border-radius: 9px;\n  background: var\(--server-tint, var\(--accent\)\);/);
  assert.match(rule(`${R} .emoji-browser .eb-rail-btn.active > .eb-rail-server`), /outline: 2px solid var\(--accent-lit\);/);
  assert.match(rule(`${R} .emoji-browser .eb-rail-btn.active`), /color: var\(--accent-lit\);/);
  assert.match(rule(`${R} :is(.emoji-browser, .picker-plain) .emoji-section`), /color: var\(--text-muted\);\n  font: 600 13px\/1 var\(--font\);\n  letter-spacing: 0;\n  text-transform: none;/);
  assert.match(rule(`${R} :is(.emoji-browser, .picker-plain) .emoji-section-add`), /height: 24px;[\s\S]*box-shadow: inset 0 0 0 1px var\(--line\);/);
  assert.match(rule(`${R} .emoji-browser .eb-cells > button`), /border-radius: 10px;/);
  assert.match(rule(`${R} .emoji-browser .eb-cells > button:focus-visible`), /background: var\(--bg-active\); box-shadow: inset 0 0 0 2px var\(--accent-lit\);/);
  assert.match(rule(`${R} .emoji-browser > .eb-foot`), /border-radius: var\(--radius-pill\);\n  background: var\(--input-bg\);\n  box-shadow: var\(--slot\);/);
  assert.match(rule(`${R} .emoji-browser .eb-foot-name`), /font: 600 13\.5px\/1 var\(--font-mono\);/);
  assert.match(rule(`${R} .picker-plain .emoji-grid[data-kind="stickers"]`), /repeat\(4, minmax\(0, 1fr\)\); gap: 8px;/);
  assert.match(rule(`${R} .picker-plain .emoji-grid button.sticker-tile`), /border-radius: 16px;/);
  assert.match(rule(`${R} .picker-plain .emoji-grid button.emoticon-tile`), /height: 40px;\n  padding: 0 14px;\n  border-radius: 12px;/);
  assert.match(rule(`${R} .picker-plain .emoticon-tile > .emoticon-name`), /color: var\(--text-faint\); font: 400 12\.5px\/1 var\(--font-mono\);/);
  assert.equal(railServerId("server:rh"), "rh");
  assert.equal(railServerId("catalog"), null);
  assert.match(glue, /tile\.style\.setProperty\("--server-tint", serverTint\(id\)\)/);
  assert.match(glue, /for \(const id of \["composer-emoji-browser", "emoji-pop-browser"\]\)/);
});

test("GIFs are two soft columns that fade out, and no GIF search is a quiet well with the app's words", () => {
  assert.match(rule(`${R} :is(.gif-picker, .pop-gifs) > .gif-grid`), /gap: 6px;[\s\S]*mask-image: linear-gradient\(black 86%, transparent\);/);
  assert.match(rule(`${R} :is(.gif-picker, .pop-gifs) .gif-item`), /border-radius: 12px;/);
  assert.match(rule(`${R} :is(.gif-picker, .pop-gifs) .gif-item:hover`.replace(":hover", ":hover,\n" + R + " :is(.gif-picker, .pop-gifs) .gif-item:focus-visible")), /outline: 2px solid var\(--accent-lit\); outline-offset: -2px;/);
  assert.match(rule(`${R} :is(.gif-picker, .pop-gifs) .gif-note::before`), /width: 56px;\n  height: 56px;\n  border-radius: 50%;\n  background: var\(--input-bg\);\n  box-shadow: var\(--slot\);/);
  assert.match(rule(`${R} :is(.gif-picker, .pop-gifs) .gif-note > .btn-link`), /color: var\(--link\); font: 600 13\.5px\/1 var\(--font\);/);
  assert.match(rule(`${R} :is(.gif-picker, .pop-gifs) > .gif-attrib`), /color: var\(--text-faint\); font: 600 12px\/1 var\(--font\);/);
  assert.ok(css.includes(`${R} :is(.gif-picker, .pop-gifs):has(> .gif-grid > .gif-note) > .gif-attrib { visibility: hidden; }`));
});

test("search results are a 460 slab with ringed scopes, accent-lit marks, a hairline pager and the quiet star", () => {
  const glue = read("../scripts/studio-float.js");
  assert.match(rule(`${R} .search-pop`), /width: 460px;/);
  assert.match(rule(`${R} .search-pop .search-pop-title`), /font: 700 17px\/1\.2 var\(--font-display\);/);
  assert.match(rule(`${R} .search-pop .search-scope-btn.active`), /outline: var\(--ring-on\);/);
  assert.match(rule(`${R} .search-pop .search-row`), /padding: 10px;\n  border-radius: 14px;/);
  assert.match(rule(`${R} .search-pop .search-row.active`), /background: var\(--bg-active\); box-shadow: inset 0 0 0 1\.5px var\(--accent-lit\);/);
  assert.match(rule(`${R} .search-pop .search-row .pin-text`), /-webkit-line-clamp: 2;/);
  assert.match(rule(`${R} .search-pop .search-hit`), /background: color-mix\(in oklab, var\(--accent\) 38%, transparent\);\n  color: var\(--header-primary\);/);
  assert.match(rule(`${R} .search-pop > .search-pager`), /border-top: 1px solid var\(--line\);/);
  assert.ok(css.includes(`${R} .search-pop > .search-pager:is(.idle, :has(> #search-newer:disabled):has(> #search-older:disabled)) { display: none; }`));
  assert.ok(css.includes(`${R} .search-pop:has(> .search-list > .search-empty) { height: auto; }`));
  assert.equal(read("../scripts/app.js").split("setSearchHead(total || !studioLook() ? pageCount(total) : searchScopeTitle(scope, ctx), searchScopeLine(scope, query, ctx));").length, 3);
  assert.match(rule(`${R} .search-pop .search-empty > .pins-empty-mark > .sstar`), /--star-size: 48px;/);
  assert.deepEqual(PICKER_KEYS, ["lightbox-open", "lightbox-save", "search-newer", "search-older"]);
  for (const id of PICKER_KEYS) assert.ok(html.includes(`id="${id}"`), id);
  assert.match(glue, /studioKeyDress\(key, "key sm", roomy\)/);
  assert.match(glue, /star\.dataset\.state = "quiet";/);
  assert.match(glue, /new MutationObserver\(studioSearchStar\)\.observe\(list, \{ childList: true \}\)/);
});

test("the picture viewer blurs the app, floats the picture and holds the source and keys in one glass tray", () => {
  assert.match(rule(`${R} .lightbox`), /background: var\(--scrim\);\n  -webkit-backdrop-filter: blur\(14px\);\n  backdrop-filter: blur\(14px\);/);
  assert.match(rule(`${R} .lightbox > img`), /border-radius: 16px;\n  box-shadow: var\(--shadow-3\)/);
  assert.match(rule(`${R} .lightbox::after`), /grid-area: 2 \/ 2 \/ 3 \/ 4;\n  align-self: stretch;\n  border-radius: var\(--radius-pill\);/);
  assert.match(rule(`${R} .lightbox > .lightbox-source.linked`), /color: var\(--link\);/);
  assert.match(rule(`${R} .lightbox .lightbox-action.key`), /border-radius: var\(--radius-pill\);/);
  assert.match(rule(`${R} body:has(> .lightbox:not(.hidden)) > .toast`), /top: 22px; bottom: auto;/);
});
