import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { STRIP_TILES, STRIP_CHIP_GAP, stripStacked, stripChipOffsets, stripIsOnline, stripPersonWord, stripOnlineHead } = require("../scripts/studio-strip.js");
const { phoneAppViewport, SLIM_VIEWPORT, SLIM_LAYOUT_WIDTH_PX, PHONE_APP_VIEWPORT, TWO_PANE_VIEWPORT } = require("../scripts/phone.js");
const { LOOK_FIT } = require("../scripts/look.js");

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio-strip.css");
const glue = read("../scripts/studio-strip.js");
const app = read("../scripts/app.js");
const STACK = 'html[data-look="studio"][data-fit="slim"][data-stack="on"]';
const selectors = [...css.replace(/@keyframes[^{]+\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, "").replace(/@media[^{]+\{/g, "").matchAll(/(^|\})\s*([^{}@]+)\{/g)]
  .flatMap(match => match[2].split(/,\n/).map(part => part.trim()).filter(Boolean));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};
const once = (text, part) => text.indexOf(part) >= 0 && text.indexOf(part) === text.lastIndexOf(part);

test("the stacked strip sheet only speaks to a slim window that stacks", () => {
  const link = '<link rel="stylesheet" id="studio-strip" href="styles/studio-strip.css" media="not all" />';
  assert.ok(once(html, link));
  assert.ok(html.includes('<link rel="stylesheet" id="studio-start" href="styles/studio-start.css" media="not all" />\n  ' + link));
  assert.ok(selectors.length > 80);
  for (const selector of selectors) assert.ok(selector.startsWith(STACK), selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
});

test("the glue loads once after the first run glue and starts once after the floating parts", () => {
  const script = '<script src="scripts/studio-strip.js"></script>';
  assert.ok(once(html, script));
  assert.ok(html.includes('<script src="scripts/studio-start.js"></script>\n  ' + script));
  const line = "  if (studioLook()) studioStrip();\n";
  assert.ok(once(app, line));
  assert.ok(app.indexOf(line) > app.indexOf("  if (studioLook()) studioFloat();\n"));
  assert.doesNotMatch(glue, /\/\/|\/\*/);
});

test("the stack is drawn 550 wide: top 96, gaps 12, Home at most 25.7 percent, the room 180 plus the picture, the bar 144", () => {
  assert.equal(LOOK_FIT.stackWidth, 550);
  assert.equal(LOOK_FIT.stackHeight, 1800);
  const root = rule(STACK);
  assert.match(root, /--st-gap: 12px;/);
  assert.match(root, /--st-bar: 144px;/);
  assert.match(root, /--st-home: calc\(var\(--st-th\) \* \.257\);/);
  assert.match(root, /--st-stream: calc\(\(100vw - 20px\) \* \.5585\);/);
  assert.match(root, /--st-room: calc\(180px \+ var\(--st-stream\)\);/);
  assert.match(rule(`${STACK} .app[data-pane]`), /grid-template-rows: 96px auto minmax\(0, 1fr\) auto;/);
  assert.match(rule(`${STACK} .app[data-pane] > :is(.sidebar, .pt-live, .pt-you)`), /max-height: var\(--st-home\);/);
  assert.match(rule(`${STACK} .app[data-pane].pt-voice-open > .pt-room`), /height: var\(--st-room\);/);
  assert.match(rule(`${STACK} .app[data-pane].pt-in-voice`), /padding-bottom: var\(--st-bar\);/);
  assert.match(rule(`${STACK} .app[data-pane]:not(.pt-in-voice) > .hzn`), /display: none;/);
  assert.match(rule(`${STACK} .app[data-pane] > .hzn .hzn-tabs`), /display: none;/);
});

test("the top tile row is the way around: Messages and Live before the servers, You last, tiles 66 wide, names at most 62", () => {
  assert.deepEqual(STRIP_TILES.map(tile => [tile.id, tile.label, tile.first]), [["messages", "Messages", true], ["live", "Live", true], ["you", "You", false]]);
  assert.match(rule(`${STACK} .app[data-pane] .rail-item`), /width: 66px;/);
  assert.match(rule(`${STACK} .app[data-pane] .rail-btn.rail-add`), /margin: 3px 7px 0;/);
  assert.match(rule(`${STACK} .app[data-pane] .rail-item .pt-name`), /max-width: 62px !important;/);
});

test("Home lays text channels on the left and the voice rooms with the people online on the right", () => {
  const home = `${STACK} .app[data-pane][data-tab="home"]:not(.dm-mode) .channel-scroll`;
  assert.match(rule(home), /grid-template-columns: repeat\(2, minmax\(0, 1fr\)\);/);
  assert.match(rule(home), /grid-auto-flow: row dense;/);
  assert.match(rule(`${home} > .channel:not(.sidebar-link)`), /height: 46px;/);
  assert.match(rule(`${home} > .voice-channel.pt-busy`), /height: 56px;/);
  assert.match(rule(`${STACK} .app[data-pane] .strip-person`), /height: 42px;/);
  assert.ok(css.includes(".section-head:has(+ .voice-channel), .strip-online-head, .strip-person) { grid-column: 2; }"));
  assert.deepEqual(stripChipOffsets([80, 60, 120]), [0, 80 + STRIP_CHIP_GAP, 140 + 2 * STRIP_CHIP_GAP]);
  assert.deepEqual(stripChipOffsets([]), []);
  assert.equal(stripOnlineHead(5), "Online now · 5");
});

test("a person online says where they are in the app's own words", () => {
  assert.equal(stripIsOnline({ presence: "online" }), true);
  assert.equal(stripIsOnline({ presence: "idle" }), true);
  assert.equal(stripIsOnline({ presence: "do_not_disturb" }), true);
  assert.equal(stripIsOnline({ presence: "invisible" }), false);
  assert.equal(stripIsOnline({}), false);
  assert.equal(stripPersonWord({ room: "Lounge", sharing: true }), "Sharing a screen");
  assert.equal(stripPersonWord({ room: "Lounge", sharing: false, status: "x" }), "In Lounge");
  assert.equal(stripPersonWord({ status: "hosting tonight", presence: "Online" }), "hosting tonight");
  assert.equal(stripPersonWord({ presence: "Idle" }), "Idle");
});

test("the glue only acts in the stack and takes back what it added", () => {
  assert.equal(stripStacked({ look: "studio", fit: "slim", stack: "on" }), true);
  assert.equal(stripStacked({ look: "studio", fit: "slim" }), false);
  assert.equal(stripStacked({ look: "studio", fit: "roomy", stack: "on" }), false);
  assert.equal(stripStacked({ fit: "slim", stack: "on" }), false);
  assert.ok(glue.includes('if (!stripStacked(root.dataset)) {\n        undo();'));
  assert.ok(glue.includes("app.dataset.stripRoom = \"on\";"));
  assert.ok(glue.includes("if (app.dataset.stripRoom) {\n      delete app.dataset.stripRoom;\n      app.classList.remove(\"pt-voice-open\");"));
});

test("an upright phone app screen wider than 550 is laid out 550 wide in the new look only", () => {
  assert.equal(SLIM_LAYOUT_WIDTH_PX, 550);
  assert.match(SLIM_VIEWPORT, /^width=550, user-scalable=no/);
  assert.equal(phoneAppViewport(800, true, true), SLIM_VIEWPORT);
  assert.equal(phoneAppViewport(669, true, true), SLIM_VIEWPORT);
  assert.equal(phoneAppViewport(393, true, true), PHONE_APP_VIEWPORT);
  assert.equal(phoneAppViewport(1280, false, true), PHONE_APP_VIEWPORT);
  assert.equal(phoneAppViewport(800, true, false), PHONE_APP_VIEWPORT);
  assert.equal(phoneAppViewport(669, true, false), TWO_PANE_VIEWPORT);
  assert.ok(once(app, "phoneAppViewport(portrait.matches ? shortSide : longSide, portrait.matches, studioLook())"));
});
