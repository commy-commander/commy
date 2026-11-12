import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { REACH_NEVER, REACH_TYPED, REACH_FIELDS, CORNER_BAND, hoverless, dropHover, lockedViewport, fieldHints, typedInto, cornerRoom } = require("../scripts/phone-reach.js");
const { touchOnly } = require("../scripts/phone-motion.js");

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-reach.css");
const code = read("scripts", "phone-reach.js");
const app = read("scripts", "app.js");
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';

const rule = selector => {
  const at = css.indexOf(`${SLIM}${selector} {`);
  assert.notEqual(at, -1, selector);
  return css.slice(at, css.indexOf("}", at));
};

test("every reach rule is for the phone layout of the new look, with tokens and no comments", () => {
  const flat = head => (/\([^()]*\)/.test(head) ? flat(head.replace(/\([^()]*\)/g, "")) : head);
  const selectors = css.split("}").map(block => block.split("{").at(-2) || "").flatMap(head => flat(head).split(",")).map(s => s.trim()).filter(Boolean);
  assert.ok(selectors.length > 30);
  for (const selector of selectors) assert.ok(selector.startsWith(SLIM), selector);
  assert.equal(/#[0-9a-fA-F]{3,8}\b/.test(css), false);
  assert.equal(css.includes("/*"), false);
  assert.equal(/\/\*|^\s*\/\//m.test(code), false);
});

test("the reach files sit after the motion files and before the off-key sheet", () => {
  const links = [...html.matchAll(/<link rel="stylesheet" id="(studio-phone[a-z-]*)"/g)].map(m => m[1]);
  assert.equal(links.indexOf("studio-phone-reach"), links.indexOf("studio-phone-motion") + 1);
  assert.equal(links.at(-1), "studio-phone-off");
  assert.match(html, /<link rel="stylesheet" id="studio-phone-reach" href="styles\/phone-reach\.css" media="not all" \/>/);
  assert.ok(html.indexOf('<script src="scripts/phone-reach.js">') > html.indexOf('<script src="scripts/phone-motion.js">'));
  assert.ok(html.indexOf('<script src="scripts/phone-reach.js">') > html.indexOf('<script src="scripts/app.js">'));
});

test("a hover look never matches once the hover part is swapped out", () => {
  assert.equal(hoverless(".key:hover"), `.key${REACH_NEVER}`);
  assert.equal(hoverless(".row:hover > .acts, .row:focus-within > .acts"), `.row${REACH_NEVER} > .acts, .row:focus-within > .acts`);
  assert.equal(hoverless(".key:is(:hover, :active)"), `.key:is(${REACH_NEVER}, :active)`);
  assert.equal(hoverless(".key:active"), ".key:active");
  assert.equal(hoverless(".hovercard"), ".hovercard");
});

test("hover rules are rewritten in place, inside groups too, and counted", () => {
  const inner = [{ selectorText: ".a:hover" }, { selectorText: ".b" }];
  const rules = [{ selectorText: ".c:hover, .c:active" }, { cssRules: inner }, { selectorText: ".d:focus" }];
  assert.equal(dropHover(rules), 2);
  assert.equal(rules[0].selectorText, `.c${REACH_NEVER}, .c:active`);
  assert.equal(inner[0].selectorText, `.a${REACH_NEVER}`);
  assert.equal(inner[1].selectorText, ".b");
  assert.equal(rules[2].selectorText, ".d:focus");
  assert.match(code, /if \(!touchOnly\(\)\) return;\n  root\.classList\.add\("touch-only"\);\n  for \(const sheet of document\.styleSheets\) dropHoverIn\(sheet\);/);
  assert.match(code, /if \(e\.target && e\.target\.tagName === "LINK"\) dropHoverIn\(e\.target\.sheet\);/);
});

test("only a phone with no mouse counts as touch only", () => {
  assert.equal(touchOnly(), false);
  globalThis.matchMedia = query => ({ matches: query === "(hover: none) and (pointer: coarse)" });
  assert.equal(touchOnly(), true);
  globalThis.matchMedia = () => ({ matches: false });
  assert.equal(touchOnly(), false);
  delete globalThis.matchMedia;
});

test("the phone app page cannot be zoomed by accident, and keeps the rest of its viewport", () => {
  assert.equal(lockedViewport("width=device-width, initial-scale=1, viewport-fit=cover"), "width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no, viewport-fit=cover");
  assert.equal(lockedViewport("width=device-width, initial-scale=1"), "width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no");
  assert.equal(lockedViewport("width=device-width, maximum-scale=5, user-scalable=yes"), "width=device-width, maximum-scale=1, user-scalable=no");
  assert.equal(lockedViewport(lockedViewport("width=device-width, viewport-fit=cover")), lockedViewport("width=device-width, viewport-fit=cover"));
});

test("a wide phone screen that is laid out at a set width is left free to scale up and fill the screen", () => {
  const { SLIM_VIEWPORT, TWO_PANE_VIEWPORT } = require("../scripts/phone.js");
  assert.equal(lockedViewport(SLIM_VIEWPORT), SLIM_VIEWPORT);
  assert.equal(lockedViewport(TWO_PANE_VIEWPORT), TWO_PANE_VIEWPORT);
  assert.doesNotMatch(lockedViewport(SLIM_VIEWPORT), /maximum-scale/);
  assert.match(lockedViewport(SLIM_VIEWPORT), /user-scalable=no/);
  assert.match(code, /if \(!root\.classList\.contains\("mobile-app"\) && !document\.body\.classList\.contains\("mobile-app"\)\) return;/);
});

test("fields in a sheet are 16 px so iOS does not zoom in on them", () => {
  assert.match(rule(' .modal.dialog :is(input:not([type="checkbox"], [type="radio"], [type="range"], [type="color"]), textarea, select)'), /font-size: 16px !important;/);
});

test("fields tell the keyboard what its return key does and when to leave words alone", () => {
  assert.equal(REACH_FIELDS.length, 5);
  assert.deepEqual(fieldHints("settings-search"), { enterkeyhint: "search", autocapitalize: "none", autocorrect: "off" });
  assert.deepEqual(fieldHints("username-input"), { enterkeyhint: "done", autocapitalize: "none", autocorrect: "off", spellcheck: "false" });
  assert.deepEqual(fieldHints("create-name-input"), { enterkeyhint: "done", autocapitalize: "words" });
  assert.deepEqual(fieldHints("signin-email"), { enterkeyhint: "next" });
  assert.deepEqual(fieldHints("signin-password"), { enterkeyhint: "go" });
  assert.deepEqual(fieldHints("message-input"), {});
  assert.deepEqual(fieldHints(""), {});
  assert.match(code, /if \(!field\.hasAttribute\(name\)\) field\.setAttribute\(name, value\);/);
});

test("nothing is picked up and dragged by a finger, and list sheets do not open the keyboard by themselves", () => {
  assert.equal((app.match(/\.draggable = !touchOnly\(\);/g) || []).length, 4);
  assert.equal(/\.draggable = true;/.test(app), false);
  assert.match(code, /document\.addEventListener\("dragstart", e => e\.preventDefault\(\), true\);/);
  assert.match(rule(".touch-only :is(img, a, .channel, .server-icon, .folder-head, .channel-group)"), /-webkit-user-drag: none;/);
  for (const id of ["forward-search", "group-search", "new-dm-search"]) assert.ok(app.includes(`if (!touchOnly()) $("${id}").focus();`), id);
});

test("small keys get a 44 px touch area without changing how they look", () => {
  assert.match(rule(" .main > .composer :is(#text-style-btn, #gif-btn) > .pt-fk::before"), /position: absolute; inset: -8px 0 -8px -10px;/);
  assert.match(rule(" .emoji-browser .eb-rail-btn::before"), /inset: -6px;/);
  assert.match(rule(" .pt-picker-tabs > button"), /min-height: 44px;/);
  assert.match(rule(' .settings-layer .settings-content input[type="range"]:not(#sensitivity-db)'), /height: 44px; margin-block: -9px;/);
  assert.match(rule(' .ctx-slider input[type="range"]'), /height: 44px;/);
  assert.match(rule(" .upload-roles > label.role-self"), /min-height: 44px;/);
  assert.match(rule(" #channel-edit-color"), /width: 44px; height: 44px;/);
  assert.match(rule(" .events-layer .events-head .events-tabs > .events-tab"), /height: 44px;/);
  assert.match(rule(" .thread-panel #thread-title"), /box-sizing: border-box; height: 44px; margin-block: -10px; padding-block: 10px;/);
});

test("rows and small keys answer a press, and a row waits a moment so a scroll does not flash it", () => {
  const rows = " :is(.person-row, .kind-option, .template-tile, .prof-server, .pin-row, .channel.sidebar-link, .dm-thread.friend-request, label.setting-row):active";
  assert.match(rule(rows), /background-color: var\(--bg-hover\); transition: background-color 0s linear 60ms;/);
  const keys = css.split("\n").find(line => line.includes(".thread-link, .eb-tone") && line.includes(":active {"));
  assert.ok(keys);
  assert.match(keys, /opacity: \.55; transition: none;/);
});

test("lists inside sheets and pages keep their scroll to themselves", () => {
  assert.match(rule(" :is(.people-list, .pins-list, .discover-body, .forum-list, .events-list, .member-scroll, .dm-list)"), /overscroll-behavior-y: contain;/);
  assert.match(rule(" :is(.search-scope, .discover-chips, .events-tabs)"), /overscroll-behavior-x: contain;/);
});

test("a tip that only works with a keyboard is not shown on the phone", () => {
  assert.match(rule(" #modal-delete-message .setting-sub"), /display: none;/);
});

test("holding a finger down never selects text, except where you type", () => {
  assert.match(rule(".touch-only :is(body, body *)"), /-webkit-user-select: none; user-select: none; -webkit-touch-callout: none;/);
  assert.match(rule('.touch-only :is(input, textarea, select, [contenteditable]:not([contenteditable="false"]), [contenteditable]:not([contenteditable="false"]) *)'), /-webkit-user-select: text; user-select: text; -webkit-touch-callout: default;/);
  assert.match(code, /document\.addEventListener\("selectstart", e => \{\n    if \(!typedInto\(e\.target\)\) e\.preventDefault\(\);\n  \}, true\);/);
  const field = { nodeType: 1, closest: selector => (selector === REACH_TYPED ? field : null) };
  const words = { nodeType: 1, closest: () => null };
  assert.equal(typedInto(field), true);
  assert.equal(typedInto({ nodeType: 3, parentElement: field }), true);
  assert.equal(typedInto(words), false);
  assert.equal(typedInto({ nodeType: 3, parentElement: words }), false);
  assert.equal(typedInto(null), false);
});

test("empty lists tell you to hold a message on the phone and to hover it with a mouse", () => {
  assert.match(app, /function messageReach\(\) \{\n  return typeof touchOnly === "function" && touchOnly\(\) \? "Hold" : "Hover";\n\}/);
  assert.equal((app.match(/`\$\{messageReach\(\)\} a message and choose /g) || []).length, 3);
  assert.equal(/"Hover a message/.test(app), false);
});

test("a covered top right corner asks for room, anything else asks for none", () => {
  const duo = { w: 466, h: 678, top: 0 };
  assert.equal(cornerRoom({ ...duo, regions: [[383, 17, 83, 145]] }), 83);
  assert.equal(cornerRoom({ ...duo, regions: [[383, 17, 83, 145], [400, 30, 40, 40]] }), 83);
  assert.equal(cornerRoom({ ...duo, regions: [] }), 0);
  assert.equal(cornerRoom(undefined), 0);
  assert.equal(cornerRoom({ ...duo, regions: [[0, 17, 83, 145]] }), 0);
  assert.equal(cornerRoom({ ...duo, regions: [[383, CORNER_BAND, 83, 145]] }), 0);
  assert.equal(cornerRoom({ ...duo, regions: [[120, 17, 346, 60]] }), 0);
  assert.equal(cornerRoom({ ...duo, regions: [[383, 17, 0, 145]] }), 0);
  assert.equal(cornerRoom({ w: 402, h: 874, top: 62, regions: [[138, 11, 126, 37]] }), 0);
  assert.equal(cornerRoom({ w: 402, h: 874, top: 62, regions: [[300, 11, 102, 40]] }), 0);
});

test("the covered corner becomes a size the phone sheets can use, scaled to the page", () => {
  assert.match(code, /root\.setAttribute\("data-corner", "right"\);\s+root\.style\.setProperty\("--corner-room", `\$\{Math\.ceil\(room \* root\.clientWidth \/ view\.w\)\}px`\);/);
  assert.match(code, /root\.removeAttribute\("data-corner"\);\s+root\.style\.removeProperty\("--corner-room"\);/);
  assert.match(code, /showCorner\(window\.screenRegions\);\s+window\.addEventListener\("screenregions", \(\) => showCorner\(window\.screenRegions\)\);/);
  const corner = '[data-corner="right"]';
  assert.match(rule(`${corner} .app[data-pane="list"] > .sidebar > :is(.pt-find, .pt-compose)`), /translate: calc\(-1 \* var\(--corner-room\)\) 0;/);
  assert.match(rule(`${corner} .app.pt-switch[data-pane="list"] .sidebar-header`), /padding-right: calc\(66px \+ var\(--corner-room\)\);/);
  assert.match(rule(`${corner} .acct .edit`), /translate: calc\(-1 \* var\(--corner-room\)\) 0;/);
  assert.match(rule(`${corner} .pt-profile .phd .save`), /margin-right: calc\(2px \+ var\(--corner-room\)\);/);
  assert.equal(css.split("\n").filter(line => line.includes("--corner-room")).every(line => line.startsWith(`${SLIM}${corner} `)), true);
});

test("the iPhone app tells the page which parts of the screen are covered", () => {
  const swift = read("..", "apple-sign-in", "ios", "Sources", "ScreenRegions.swift");
  const plugin = read("..", "apple-sign-in", "ios", "Sources", "AppleSignInPlugin.swift");
  assert.match(swift, /reservedRegions\(kind: \.occlusion\)/);
  assert.match(swift, /window\.screenRegions=/);
  assert.match(swift, /new Event\('screenregions'\)/);
  assert.equal(/^\s*\/\//m.test(swift), false);
  assert.match(plugin, /override func load\(webview: WKWebView\) \{\s+ScreenRegions\.watch\(webview\)/);
});

test("the iPhone app lays the page out over the whole screen, safe areas included", () => {
  const swift = read("..", "apple-sign-in", "ios", "Sources", "ScreenRegions.swift");
  assert.match(swift, /DispatchQueue\.main\.async \{\s+page\.scrollView\.contentInsetAdjustmentBehavior = \.never/);
  assert.match(html, /<meta name="viewport" content="[^"]*viewport-fit=cover/);
});
