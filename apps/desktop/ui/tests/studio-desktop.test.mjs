import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { lookSearch } = require("../scripts/look.js");
const { STUDIO_ROOM_SVG, studioRoomMarkup, mountStudioRoom } = require("../scripts/studio-room.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const classic = read("../styles/styles.css");
const css = read("../styles/studio.css");

const SWITCH = 'html[data-look="studio"]';
const NOT_SLIM = ':not([data-fit="slim"])';

function selectors(sheet) {
  const found = [];
  const body = sheet.replace(/@keyframes[^{]+\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, "");
  for (const match of body.matchAll(/([^{}]+)\{/g)) {
    const head = match[1].trim();
    if (!head || head.startsWith("@")) continue;
    for (const part of head.split(",")) found.push(part.trim());
  }
  return found;
}

function fakeDocument(look) {
  const made = [];
  const body = { prepend: node => made.unshift(node) };
  return {
    made,
    body,
    documentElement: { dataset: look ? { look } : {} },
    querySelector: () => made[0] || null,
    createElement: () => ({ className: "", attrs: {}, innerHTML: "", setAttribute(name, value) { this.attrs[name] = value; } }),
  };
}

test("every desktop rule waits for the new look switch", () => {
  const list = selectors(css);
  assert.ok(list.length > 200);
  for (const selector of list) assert.ok(selector.startsWith(SWITCH), selector);
});

test("only the chat bar and the room's off state apply in a slim window", () => {
  const anyFit = selectors(css).filter(selector => !selector.includes(NOT_SLIM) && !selector.includes('[data-fit="compact"]'));
  assert.ok(anyFit.length > 20);
  for (const selector of anyFit) assert.match(selector, /\.composer|\.studio-room/, selector);
});

test("the desktop stylesheet carries no comments and no fixed colours of its own", () => {
  assert.equal(css.includes("/*"), false);
  const colours = css.match(/#[0-9a-fA-F]{3,8}\b/g) || [];
  assert.deepEqual([...new Set(colours)], ["#000"]);
});

test("the desktop stylesheet is linked switched off, between the shared parts and the phone layout", () => {
  const parts = html.indexOf('<link rel="stylesheet" id="studio-parts" href="styles/studio-parts.css" media="not all" />');
  const desktop = html.indexOf('<link rel="stylesheet" id="studio-desktop" href="styles/studio.css" media="not all" />');
  const phone = html.indexOf('id="studio-phone"');
  assert.ok(parts > 0 && desktop > parts);
  assert.ok(phone === -1 || phone > desktop);
});

test("the rail's left group sits in one tray that the classic look ignores", () => {
  const start = html.indexOf('<div class="rail-tray">');
  const spacer = html.indexOf('<div class="rail-spacer">');
  assert.ok(start > 0 && spacer > start);
  const tray = html.slice(start, spacer);
  for (const id of ["home-btn", "server-list", "add-server-btn", "discover-btn"]) assert.ok(tray.includes(`id="${id}"`), id);
  assert.ok(tray.includes('class="rail-divider"'));
  assert.equal(tray.includes("user-panel"), false);
  assert.match(classic, /\.rail-tray \{ display: contents; \}/);
});

test("the chat bar gains a Send key that the classic look hides", () => {
  const start = html.indexOf('<div class="composer">');
  const end = html.indexOf("</main>", start);
  const bar = html.slice(start, end);
  assert.ok(bar.includes('id="send-btn"'));
  assert.ok(bar.indexOf('id="send-btn"') > bar.indexOf('id="emoji-btn"'));
  assert.match(classic, /\.composer-send \{ display: none; \}/);
});

test("Settings has a New Look switch", () => {
  assert.ok(html.includes('id="new-look-toggle"'));
  assert.ok(html.includes('data-i18n="appearance.newLook"'));
});

test("choosing a look drops any look word from the address and keeps the rest", () => {
  assert.equal(lookSearch("?studio"), "");
  assert.equal(lookSearch("?mobile&studio"), "?mobile");
  assert.equal(lookSearch("?classic=1&mobile"), "?mobile");
  assert.equal(lookSearch("?mobile"), "?mobile");
  assert.equal(lookSearch(""), "");
});

test("the room scene is one hidden drawing with no comments", () => {
  assert.equal(studioRoomMarkup(), STUDIO_ROOM_SVG);
  assert.ok(STUDIO_ROOM_SVG.startsWith("<svg "));
  assert.ok(STUDIO_ROOM_SVG.includes("aria-hidden='true'"));
  assert.equal(STUDIO_ROOM_SVG.includes("<!--"), false);
  const ids = [...STUDIO_ROOM_SVG.matchAll(/ id='([^']+)'/g)].map(match => match[1]);
  assert.equal(new Set(ids).size, ids.length);
  for (const id of ids) assert.ok(id.startsWith("rm-"), id);
});

test("the room is only built when the new look is on, and only once", () => {
  const off = fakeDocument(null);
  assert.equal(mountStudioRoom(off), null);
  assert.equal(off.made.length, 0);
  const on = fakeDocument("studio");
  const room = mountStudioRoom(on);
  assert.equal(room.className, "studio-room");
  assert.equal(room.attrs["aria-hidden"], "true");
  assert.equal(room.innerHTML, STUDIO_ROOM_SVG);
  assert.equal(mountStudioRoom(on), room);
  assert.equal(on.made.length, 1);
});
