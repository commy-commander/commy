import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import vm from "node:vm";

const require = createRequire(import.meta.url);
const { lookSearch } = require("../scripts/look.js");
const { STUDIO_ROOM_SVG, studioRoomMarkup, mountStudioRoom, mountStudioStar, studioStarState } = require("../scripts/studio-room.js");
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

test("the roomy stage grows with the window between 1180 and 1920 wide", () => {
  const sheet = css.replace(/\r\n/g, "\n");
  const fluid = {
    "--st-ml": "clamp(24px, calc(5.95vw - 46px), 68px)",
    "--st-mr": "clamp(26px, calc(3.5vw - 15px), 52px)",
    "--st-gap": "clamp(26px, calc(1.08vw + 13.2px), 34px)",
    "--st-wing": "clamp(232px, calc(9.2vw + 123px), 300px)",
    "--st-side": "clamp(228px, calc(6vw + 157px), 272px)",
    "--st-lean": "clamp(8px, calc(var(--st-ml) - 14px), 26px)",
  };
  for (const [name, value] of Object.entries(fluid)) assert.ok(sheet.includes(`${name}: ${value};`), name);
  const at = (value, width) => {
    const [, low, slope, base, high] = value.match(/clamp\((\d+)px, calc\(([\d.]+)vw ([+-] [\d.]+)px\), (\d+)px\)/);
    return Math.round(Math.min(Number(high), Math.max(Number(low), (Number(slope) * width) / 100 + Number(base.replace(" ", "")))));
  };
  assert.deepEqual([1180, 1440, 1920, 2560].map(width => at(fluid["--st-ml"], width)), [24, 40, 68, 68]);
  assert.deepEqual([1180, 1440, 1920, 2560].map(width => at(fluid["--st-side"], width)), [228, 243, 272, 272]);
  assert.deepEqual([1180, 1440, 1920, 2560].map(width => at(fluid["--st-wing"], width)), [232, 255, 300, 300]);
  assert.match(sheet, /\.messages \.msg-content \{ max-width: 1000px; \}/);
  assert.match(sheet, /@media \(min-width: 1340px\) and \(max-width: 1400px\)/);
});

test("only the chat bar and the off states of the room and the star apply in a slim window", () => {
  const anyFit = selectors(css).filter(selector => !selector.includes(NOT_SLIM) && !selector.includes('[data-fit="compact"]'));
  assert.ok(anyFit.length > 20);
  for (const selector of anyFit) assert.match(selector, /\.composer|\.studio-room|\.studio-star/, selector);
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

function fakeHeader(look, withActions = true) {
  const placed = [];
  const node = () => ({ className: "", attrs: {}, children: [], tabIndex: 0, setAttribute(name, value) { this.attrs[name] = value; }, appendChild(child) { this.children.push(child); } });
  const actions = { before: key => placed.push(key) };
  return {
    placed,
    documentElement: { dataset: look ? { look } : {} },
    createElement: node,
    querySelector: selector => {
      if (selector === ".studio-star") return placed[0] || null;
      if (selector === ".studio-star .star") return placed[0] ? placed[0].children[0] : null;
      return withActions ? actions : null;
    },
  };
}

function loadStar() {
  const window = { document: { currentScript: null } };
  vm.runInNewContext(read("../scripts/star.js"), { window });
  return window.Star;
}

test("the star stylesheet and script load switched off, ahead of the parts that place the star", () => {
  const parts = html.indexOf('id="studio-parts"');
  const star = html.indexOf('<link rel="stylesheet" id="studio-star" href="styles/star.css" media="not all" />');
  const desktop = html.indexOf('id="studio-desktop"');
  assert.ok(star > parts && desktop > star);
  const script = html.indexOf('<script src="scripts/star.js"></script>');
  assert.ok(script > 0 && html.indexOf('<script src="scripts/studio-room.js"></script>') > script);
});

test("the star files are the brand kit's drawing with the comments taken out", () => {
  const source = read("../scripts/star.js");
  assert.equal(/\/\*|^\s*\/\//m.test(source), false);
  assert.equal(read("../styles/star.css").includes("/*"), false);
  const star = loadStar();
  assert.deepEqual([...star.states], ["idle", "near", "press", "sent", "waiting", "quiet", "unimpressed", "surprised", "working", "off"]);
  const art = star.svg();
  assert.ok(art.startsWith('<svg class="star-art" viewBox="0 0 100 100"'));
  assert.equal((art.match(/class="star-dot /g) || []).length, 3);
  assert.equal(art.includes("<!--"), false);
});

test("the star is placed once, in front of the header keys, only when the new look is on", () => {
  const mounted = [];
  const star = { mount: node => mounted.push(node) };
  const off = fakeHeader(null);
  assert.equal(mountStudioStar(off, star), null);
  assert.equal(mountStudioStar(fakeHeader("studio"), null), null);
  assert.equal(mountStudioStar(fakeHeader("studio", false), star), null);
  assert.equal(mounted.length, 0);
  const on = fakeHeader("studio");
  const key = mountStudioStar(on, star);
  assert.equal(key.className, "studio-star");
  assert.equal(key.type, "button");
  assert.equal(key.tabIndex, -1);
  assert.equal(key.attrs["aria-hidden"], "true");
  assert.equal(key.children[0].className, "star");
  assert.equal(key.children[0].attrs["data-state"], "idle");
  assert.deepEqual(mounted, [key.children[0]]);
  assert.equal(mountStudioStar(on, star), key);
  assert.equal(on.placed.length, 1);
});

test("the star takes a state only once it is on the page", () => {
  const calls = [];
  const star = { mount: () => {}, set: (node, state) => calls.push([node.className, state]) };
  assert.equal(studioStarState(fakeHeader("studio"), star, "sent"), false);
  const on = fakeHeader("studio");
  mountStudioStar(on, star);
  assert.equal(studioStarState(on, null, "sent"), false);
  assert.equal(studioStarState(on, star, "sent"), true);
  assert.deepEqual(calls, [["star", "sent"]]);
});

test("message text, code and lists take the guide's sizes from the chat text size", () => {
  const desk = `${SWITCH}${NOT_SLIM}`;
  assert.ok(css.includes(`${desk} .msg-content { font-size: calc(var(--chat-font-size) * .9375); line-height: calc(var(--chat-font-size) * 1.375); }`));
  assert.ok(css.includes(`${desk} .msg-content h2.md-heading { font-size: calc(var(--chat-font-size) * 1.125); line-height: calc(var(--chat-font-size) * 1.625); }`));
  assert.ok(css.includes(`${desk} .msg-content .md-code code { font-size: calc(var(--chat-font-size) * .8125); }`));
  assert.ok(css.includes(`${desk} .msg-content ul.md-list { padding: 0; list-style: none; }`));
  assert.ok(css.includes(`${desk} .md-quote::before {`));
});

test("a picture in chat shows without its file card and the server strip keeps its divider", () => {
  const desk = `${SWITCH}${NOT_SLIM}`;
  assert.ok(css.includes(`${desk} .msg .attachment:has(+ img.chat-image) { display: none; }`));
  assert.ok(css.includes(`${desk} .rail-add:not(.rail-discover)::before {`));
  assert.ok(css.includes(`${desk} .server-list {\n  flex: 0 1 auto;\n  gap: 8px;`));
  assert.ok(!css.includes(`${desk} .rail-tray button svg.ai .i-on`));
});
