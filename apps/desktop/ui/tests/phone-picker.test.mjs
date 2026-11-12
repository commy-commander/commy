import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-picker.css");
const glue = read("scripts", "phone-picker.js");
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';
const { PICKER_TABS, GIF_OFF_WORDS, pickerTabFor, gifSearchOff } = require("../scripts/phone-picker.js");

const rule = selector => {
  const at = css.indexOf(`${SLIM} ${selector} {`);
  assert.notEqual(at, -1, selector);
  return css.slice(at, css.indexOf("}", at));
};

test("every picker rule is for the phone app in the new look, with tokens and no comments", () => {
  const flat = css.replace(/@media \([a-z-]+: [a-z]+\) \{/g, "");
  const selectors = flat.split("}").map(block => block.split("{")[0]).flatMap(head => head.split(/,(?![^(]*\))/)).map(s => s.trim()).filter(Boolean);
  assert.ok(selectors.length > 40);
  for (const selector of selectors) assert.ok(selector.startsWith(SLIM), selector);
  assert.equal(/#[0-9a-fA-F]{3,8}\b/.test(css), false);
  assert.equal(css.includes("/*"), false);
  assert.equal(/\/\/|\/\*/.test(glue.replace(/https?:\/\//g, "")), false);
});

test("hover only lights where there is a pointer", () => {
  const outside = css.slice(0, css.indexOf("@media (hover: hover)"));
  assert.equal(/:hover/.test(outside), false);
  const inside = css.slice(css.indexOf("@media (hover: hover)"), css.indexOf("@media (prefers-reduced-motion: reduce)"));
  assert.match(inside, /\.pt-picker-tabs > button:not\(\.on\):hover/);
  assert.match(inside, /\.gif-item:hover \{ outline: 2px solid var\(--accent-lit\)/);
});

test("the picker file loads with the phone files, before the off look, which stays last", () => {
  assert.match(html, /<link rel="stylesheet" id="studio-phone-plus" href="styles\/phone-plus\.css" media="not all" \/>\n(\s*<link rel="stylesheet" id="studio-phone-[a-z-]+" href="styles\/phone-[a-z-]+\.css" media="not all" \/>\n)*\s*<link rel="stylesheet" id="studio-phone-picker" href="styles\/phone-picker\.css" media="not all" \/>/);
  const links = [...html.matchAll(/id="(studio-phone[a-z-]*)"/g)].map(m => m[1]);
  assert.equal(links.at(-1), "studio-phone-off");
  assert.ok(links.indexOf("studio-phone-picker") > links.indexOf("studio-phone-plus"));
  assert.match(html, /<script src="scripts\/phone-plus\.js"><\/script>\n(\s*<script src="scripts\/phone-[a-z-]+\.js"><\/script>\n)*\s*<script src="scripts\/phone-picker\.js"><\/script>/);
});

test("one tab row holds the app's three kinds and GIFs, in the app's words", () => {
  assert.deepEqual(PICKER_TABS.map(tab => tab.label), ["Emoji", "Stickers", "Emoticons", "GIFs"]);
  assert.deepEqual(PICKER_TABS.map(tab => tab.kind), ["emojis", "stickers", "emoticons", "gifs"]);
  for (const tab of PICKER_TABS.slice(0, 3)) assert.match(html, new RegExp(`data-kind="${tab.kind}"[^>]*>${tab.label}<`));
  assert.match(html, /id="gif-btn" title="GIFs"/);
  assert.equal(pickerTabFor(true, "stickers"), "gifs");
  assert.equal(pickerTabFor(false, "stickers"), "stickers");
  assert.equal(pickerTabFor(false, "nothing"), "emojis");
  assert.match(glue, /tab\.hidden = gifKey\.classList\.contains\("hidden"\) && !pickerGifOpen\(\);/);
  assert.match(rule(".pt-picker-sheet > .pt-picker-tabs"), /display: flex;[\s\S]*padding: 4px;[\s\S]*border-radius: var\(--radius-pill\);\n  background: var\(--input-bg\);\n  box-shadow: var\(--slot\);/);
  assert.match(rule(".pt-picker-tabs > button"), /flex: 1 1 0;[\s\S]*height: 40px;/);
  assert.match(rule(".pt-picker-tabs > button.on"), /background: var\(--bg-active\);\n  color: var\(--header-primary\);\n  outline: var\(--ring-on\);/);
  assert.match(rule(".pt-picker-sheet > .emoji-picker > .picker-tabs"), /display: none;/);
});

test("the tabs reuse the app's own pickers rather than new picker logic", () => {
  assert.match(glue, /if \(!pickerGifOpen\(\)\) openGifPicker\(""\);/);
  assert.match(glue, /if \(pickerGifOpen\(\)\) closeGifPicker\(\);\n  if \(!pickerEmojiOpen\(\)\) \$\("emoji-btn"\)\.click\(\);/);
  assert.match(glue, /\$\("picker-tabs"\)\.querySelector\(`button\[data-kind="\$\{kind\}"\]`\)/);
  assert.match(glue, /sheet\.append\(emoji, gif\);/);
  assert.match(glue, /parent\.insertBefore\(node, next && next\.parentNode === parent \? next : null\)/);
  assert.equal(/function (openGifPicker|closeGifPicker|setPickerKind|renderGifResults|sendGif)\b/.test(glue), false);
});

test("the sheet is a tall glass sheet with a grab, flush to the bottom and above the safe area", () => {
  const sheet = rule(".pt-picker > .pt-picker-sheet");
  assert.match(sheet, /bottom: var\(--keyboard-height, 0px\);/);
  assert.match(sheet, /height: min\(640px, calc\(var\(--app-height, 100dvh\) - env\(safe-area-inset-top\) - 56px\)\);/);
  assert.match(sheet, /padding: 30px 18px var\(--hz-safe\);/);
  assert.match(sheet, /border-radius: var\(--radius-sheet\) var\(--radius-sheet\) 0 0;/);
  assert.match(sheet, /var\(--sheet-bg\);/);
  assert.match(rule(".pt-picker > .pt-picker-scrim"), /background: var\(--scrim\);/);
  assert.match(rule(".pt-picker-sheet > .pt-picker-grab"), /width: 132px;\n  height: 30px;/);
  assert.match(rule(".pt-picker-sheet > .pt-picker-grab::before"), /width: 38px;\n  height: 5px;\n  border-radius: 3px;\n  background: var\(--line-strong\);/);
});

test("Back, a swipe down on the grab and the scrim close it", () => {
  assert.match(glue, /layerStep\("ptPicker", open, phonePicker\.step, history\.state\)/);
  assert.match(glue, /history\.pushState\(\{ \.\.\.\(history\.state \|\| \{ pane: phonePane\(\) \}\), ptPicker: true \}, ""\)/);
  assert.match(glue, /if \(\(history\.state && history\.state\.ptPicker\) \|\| !pickerShown\(\)\) return;/);
  assert.match(glue, /scrim\.onclick = closePickerSheet;/);
  assert.match(glue, /if \(!cancelled && sheetSettles\(drag\.dy, dragSpeed\(drag\.samples, e\.timeStamp\)\.y, SWIPE\.sheetAt\)\) closePickerSheet\(\);/);
  assert.match(glue, /sheet\.style\.translate = `0 \$\{sheetPull\(drag\.dy\)\}px`;/);
});

test("group G parts at phone width: capsule search, round tone key, rail, cells of at least 40", () => {
  assert.match(rule(".pt-picker-sheet .eb-search"), /height: 44px;[\s\S]*border-radius: var\(--radius-pill\);/);
  assert.match(rule(".pt-picker-sheet .eb-tone"), /width: 44px;\n  height: 44px;\n  border-radius: 50%;\n  background: var\(--glass-key\);/);
  assert.match(rule(".pt-picker-sheet .eb-rail"), /flex: 0 0 44px;[\s\S]*border-right: 1px solid var\(--line\);/);
  assert.match(rule(".pt-picker-sheet .eb-cells"), /grid-template-columns: repeat\(auto-fill, minmax\(40px, 1fr\)\);/);
  assert.match(rule(".pt-picker-sheet .eb-rail-server"), /color-mix\(in srgb, var\(--c\) 26%, var\(--bg-raised\)\)[\s\S]*color: color-mix\(in srgb, var\(--c\) 45%, var\(--white\)\);/);
  assert.match(css, /\[data-theme="light"\] \.pt-picker-sheet \.eb-rail-server \{ color: color-mix\(in srgb, var\(--c\) 50%, var\(--ink-strong\)\); \}/);
  assert.match(rule(".pt-picker-sheet .emoji-section"), /color: var\(--text-muted\);\n  font: 600 13px\/1 var\(--font\);[\s\S]*text-transform: none;/);
  assert.match(rule(".pt-picker-sheet .picker-plain .emoji-grid[data-kind=\"stickers\"]"), /repeat\(4, minmax\(0, 1fr\)\)/);
  assert.match(rule(".pt-picker-sheet .emoji-browser > .eb-foot"), /display: none;/);
});

test("GIFs are two columns of rounded tiles, KLIPY is faint on the right, and no search shows the well", () => {
  assert.match(rule(".pt-picker-sheet .gif-picker .gif-item"), /border-radius: 12px;/);
  assert.match(rule(".pt-picker-sheet .gif-picker .gif-col"), /gap: 6px;/);
  assert.match(rule(".pt-picker-sheet .gif-picker > .gif-attrib"), /color: var\(--text-faint\);\n?\s*font: 600 12px\/1 var\(--font\);\s*text-align: right;/);
  assert.match(rule(".pt-picker-sheet .gif-picker .gif-note::before"), /width: 56px;\n  height: 56px;\n  border-radius: 50%;\n  background: var\(--input-bg\);/);
  assert.match(rule(".pt-picker-sheet .gif-picker .gif-note > .btn-link"), /color: var\(--link\);/);
  assert.match(rule(".pt-picker-sheet .gif-picker.pt-gif-off > .gif-attrib"), /visibility: hidden;/);
  assert.equal(gifSearchOff(`${GIF_OFF_WORDS} Someone who manages this server can add a GIF key in Server Settings, then Integrations. Open GIF settings`), true);
  assert.equal(gifSearchOff("Searching..."), false);
  assert.equal(gifSearchOff("Could not load GIFs: down. Try again"), false);
  assert.ok(read("scripts", "app.js").includes(`note.textContent = "${GIF_OFF_WORDS} `));
});

test("the picker leaves the desktop picker and the off look alone", () => {
  assert.equal(read("styles", "studio-float.css").includes("pt-picker"), false);
  assert.equal(read("scripts", "studio-float.js").includes("pt-picker"), false);
  assert.match(glue, /if \(slim && !phonePicker\.homes\)/);
  assert.match(glue, /\} else if \(!slim && phonePicker\.homes\) \{/);
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{\n  html\[data-look="studio"\]\[data-fit="slim"\] \.pt-picker > \.pt-picker-sheet \{ animation: none; transition: none; \}/);
});
