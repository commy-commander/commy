import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { MOTION, MOVES, PRESSABLE, dragSpeed, keepSamples, swipeSettles, backSettles, sheetSettles, settleMs, viewportFrame, fullHeightAfter, railFades, railReveal, chatBarAfter, moveKind, gripOf, viewHolding, pinchScale, panWithin, pullFade } = require("../scripts/phone-motion.js");
const { LONG_PRESS } = require("../scripts/phone.js");

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-motion.css");
const touchCss = read("styles", "phone-touch.css");
const base = read("styles", "styles.css");
const code = read("scripts", "phone-motion.js");
const glue = read("scripts", "studio-phone.js");
const app = read("scripts", "app.js");
const html = read("index.html");
const viewer = read("scripts", "phone-viewer.js");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';

const rule = selector => {
  const at = css.indexOf(`${SLIM}${selector} {`);
  assert.notEqual(at, -1, selector);
  return css.slice(at, css.indexOf("}", at));
};

test("every motion rule is for the phone layout of the new look, with tokens and no comments", () => {
  const selectors = css.split("}").map(block => block.split("{").at(-2) || "").flatMap(head => head.replace(/\([^()]*\)/g, "()").replace(/\([^()]*\)/g, "()").split(",")).map(s => s.trim()).filter(Boolean);
  const plain = selectors.filter(s => !/^(@keyframes|@media|from|to|\d+%)/.test(s));
  assert.ok(plain.length > 30);
  for (const selector of plain) assert.ok(selector.startsWith(SLIM), selector);
  assert.equal(/#[0-9a-fA-F]{3,8}\b/.test(css), false);
  assert.equal(css.includes("/*"), false);
  assert.equal(/\/\/|\/\*/.test(code), false);
});

test("the motion files sit with the other phone files, before the off-key sheet", () => {
  const links = [...html.matchAll(/<link rel="stylesheet" id="(studio-phone[a-z-]*)"/g)].map(m => m[1]);
  assert.ok(links.includes("studio-phone-motion"));
  assert.ok(links.indexOf("studio-phone-motion") > links.indexOf("studio-phone"));
  assert.equal(links.at(-1), "studio-phone-off");
  assert.match(html, /<script src="scripts\/phone-touch\.js"><\/script>\n\s*<script src="scripts\/phone-motion\.js"><\/script>/);
});

test("the speed of a drag is read from the last tenth of a second only", () => {
  const samples = [{ t: 0, x: 0, y: 0 }, { t: 400, x: 10, y: 0 }, { t: 450, x: 40, y: 5 }, { t: 500, x: 70, y: 10 }];
  assert.deepEqual(dragSpeed(samples, 500), { x: 0.6, y: 0.1 });
  assert.deepEqual(dragSpeed(samples, 700), { x: 0, y: 0 });
  assert.deepEqual(dragSpeed([{ t: 10, x: 5, y: 5 }], 20), { x: 0, y: 0 });
  assert.deepEqual(dragSpeed([{ t: 10, x: 5, y: 5 }, { t: 10, x: 50, y: 5 }], 10), { x: 0, y: 0 });
  let kept = [];
  for (let t = 0; t <= 1000; t += 10) kept = keepSamples(kept, { t, x: t, y: 0 });
  assert.equal(kept[0].t, 1000 - MOTION.speedWindowMs * 2);
  assert.equal(kept.at(-1).t, 1000);
});

test("a swipe lands by distance or by a flick, and a flick the other way always takes it back", () => {
  assert.equal(swipeSettles(100, 0, 100), true);
  assert.equal(swipeSettles(99, 0, 100), false);
  assert.equal(swipeSettles(MOTION.flickTravel, MOTION.flick, 100), true);
  assert.equal(swipeSettles(MOTION.flickTravel - 1, MOTION.flick, 100), false);
  assert.equal(swipeSettles(300, -MOTION.flick, 100), false);
  assert.equal(backSettles(129, 0, 390), true);
  assert.equal(backSettles(128, 0, 390), false);
  assert.equal(backSettles(30, 0.6, 390), true);
  assert.equal(backSettles(200, -0.6, 390), false);
  assert.equal(sheetSettles(72, 0, 72), true);
  assert.equal(sheetSettles(MOTION.sheetFlickTravel, MOTION.sheetFlick, 72), true);
  assert.equal(sheetSettles(10, 2, 72), false);
});

test("what is left of a move takes the time the finger's speed asks for, within limits", () => {
  assert.equal(settleMs(300, 0), MOTION.paneMs);
  assert.equal(settleMs(300, 5), 140);
  assert.equal(settleMs(100, 0.8), 200);
  assert.equal(settleMs(100, -0.8), 200);
  assert.equal(settleMs(300, 0.1), MOTION.paneMs);
  assert.equal(settleMs(300, 5, MOTION.paneMs, MOTION.paneMinMs), MOTION.paneMinMs);
});

test("the keyboard frame is what the visible area no longer covers, wherever the system pans the page", () => {
  assert.deepEqual(viewportFrame({ fullHeight: 852, layoutHeight: 852, visibleHeight: 516, offsetTop: 0 }), { height: 516, top: 0, keyboard: 336, lift: 336 });
  assert.deepEqual(viewportFrame({ fullHeight: 852, layoutHeight: 852, visibleHeight: 516, offsetTop: 140 }), { height: 516, top: 140, keyboard: 336, lift: 336 });
  assert.deepEqual(viewportFrame({ fullHeight: 852, layoutHeight: 852, visibleHeight: 516, offsetTop: 900 }), { height: 516, top: 336, keyboard: 336, lift: 336 });
  assert.deepEqual(viewportFrame({ fullHeight: 800, layoutHeight: 500, visibleHeight: 500, offsetTop: 0 }), { height: 500, top: 0, keyboard: 300, lift: 0 });
  assert.deepEqual(viewportFrame({ fullHeight: 852, layoutHeight: 852, visibleHeight: 852, offsetTop: 0 }), { height: 852, top: 0, keyboard: 0, lift: 0 });
  assert.deepEqual(viewportFrame({ fullHeight: 852, layoutHeight: 852, visibleHeight: 800, offsetTop: 12 }), { height: 800, top: 0, keyboard: 0, lift: 0 });
  assert.deepEqual(viewportFrame({ fullHeight: 852, visibleHeight: 516, offsetTop: 0 }), { height: 516, top: 0, keyboard: 336, lift: 336 });
});

test("the full height is remembered while typing and measured again after a turn", () => {
  assert.equal(fullHeightAfter(0, 852, 852, false), 852);
  assert.equal(fullHeightAfter(852, 516, 516, false), 852);
  assert.equal(fullHeightAfter(852, 393, 393, true), 393);
  assert.equal(fullHeightAfter(393, 852, 852, false), 852);
});

test("the app follows the keyboard: it shrinks at once, follows a panned page, and keeps the newest message in view", () => {
  assert.match(app, /const frame = viewportFrame\(\{ fullHeight: seen\.full, layoutHeight: window\.innerHeight, visibleHeight: view\.height, offsetTop: view\.offsetTop \}\);/);
  assert.match(app, /root\.style\.setProperty\("--app-top", `\$\{frame\.top\}px`\);\n\s+root\.style\.setProperty\("--keyboard-height", `\$\{frame\.lift\}px`\);/);
  assert.match(app, /root\.classList\.toggle\("viewport-shifted", frame\.top > 0\);\n\s+for \(const log of atEnd\) log\.scrollTop = log\.scrollHeight;/);
  assert.match(app, /if \(e\.target\.matches && e\.target\.matches\(TYPING_FIELDS\)\) guessOpen\(\);/);
  assert.match(app, /seen\.softKeys = seen\.full - view\.height >= MOTION\.keyboardMin;/);
  assert.ok(base.includes("html.viewport-shifted body.mobile-app { translate: 0 var(--app-top, 0px); }"));
});

test("the server strip fades where more servers wait and brings a chosen one fully into view", () => {
  assert.deepEqual(railFades(0, 390, 760), { left: false, right: true });
  assert.deepEqual(railFades(200, 390, 760), { left: true, right: true });
  assert.deepEqual(railFades(370, 390, 760), { left: true, right: false });
  assert.deepEqual(railFades(0, 390, 390), { left: false, right: false });
  assert.equal(railReveal(100, 74, 0, 390, 760), 0);
  assert.equal(railReveal(380, 74, 0, 390, 760), 74);
  assert.equal(railReveal(84, 74, 200, 390, 760), 74);
  assert.equal(railReveal(700, 74, 0, 390, 760), 370);
  assert.equal(railReveal(4, 74, 60, 390, 760), 0);
  assert.match(glue, /const fades = studioSlim\(\) \? railFades\(rail\.scrollLeft, rail\.clientWidth, rail\.scrollWidth\) : \{ left: false, right: false \};/);
  assert.match(glue, /rail\.addEventListener\("scroll", paintRailFades, \{ passive: true \}\);\n\s+new ResizeObserver\(paintRailFades\)\.observe\(rail\);/);
  assert.match(glue, /const next = railReveal\(left, box\.width, rail\.scrollLeft, rail\.clientWidth, rail\.scrollWidth\);/);
  const strip = touchCss.slice(touchCss.indexOf(`${SLIM} .app[data-pane="list"] > .rail {`));
  assert.match(strip.slice(0, strip.indexOf("}")), /overflow-x: auto;\n  overflow-y: hidden;\n  overscroll-behavior-x: contain;\n  touch-action: pan-x;/);
  assert.ok(touchCss.includes(`${SLIM} .app[data-pane="list"] > .rail::after { content: ""; flex: none; width: 10px; height: 1px; }`));
  assert.ok(touchCss.includes(`${SLIM} .app[data-pane="list"] > .rail.pt-more-left.pt-more-right { -webkit-mask-image: linear-gradient(90deg, transparent, black 40px, black calc(100% - 40px), transparent);`));
});

test("going deeper pushes a screen in and going back pops it out", () => {
  assert.deepEqual(MOVES, ["push", "pop"]);
  assert.equal(moveKind("list", "chat"), "push");
  assert.equal(moveKind("chat", "people"), "push");
  assert.equal(moveKind("people", "chat"), "pop");
  assert.equal(moveKind("chat", "list"), "pop");
  assert.equal(moveKind("chat", "chat"), null);
  assert.match(app, /phonePaneNext = pane;\n\s+runPhoneMove\(moveKind\(was, pane\), \(\) => \{\n\s+\$\("app"\)\.dataset\.pane = pane;/);
  assert.match(app, /return phonePaneNext \|\| \$\("app"\)\.dataset\.pane \|\| "list";/);
  assert.match(code, /const shown = MOVES\.includes\(kind\) && !calm && root\.dataset\.fit === "slim" && root\.dataset\.look === "studio" && !document\.hidden;/);
  assert.match(code, /if \(typeof document\.startViewTransition !== "function"\) \{\n\s+apply\(\);\n\s+root\.dataset\.move = `\$\{kind\}-in`;/);
  assert.match(rule('[data-move="push"]::view-transition-new(root)'), /animation: pt-move-over-in var\(--pt-move-ms, 340ms\) var\(--ease-settle\) both;/);
  assert.match(rule('[data-move="pop"]::view-transition-old(root)'), /z-index: 1; clip-path: inset\(0 0 0 var\(--pt-pop-from, 0px\)\); animation: pt-move-over-out/);
  assert.match(rule("[data-move]::view-transition"), /pointer-events: none;/);
  assert.match(css, /@keyframes pt-move-over-out \{\n  to \{ translate: calc\(100% - var\(--pt-pop-from, 0px\)\) 0; \}\n\}/);
});

test("a pull from the edge carries the screen with the finger, then hands over to the pop from where it was let go", () => {
  assert.match(glue, /const move = \{ from: dx, ms: settleMs\(left, speed, MOTION\.paneMs, MOTION\.paneMinMs\), land: rest \};\n\s+primePhoneMove\(move\);\n\s+phoneStep\("back"\);/);
  assert.match(glue, /if \(swipe\.profile && profileDirty\(\)\) \{/);
  assert.match(code, /root\.style\.setProperty\("--pt-pop-from", `\$\{Math\.max\(0, Math\.round\(\(next && next\.from\) \|\| 0\)\)\}px`\);/);
});

test("swipes paint once a frame, read the finger's speed, and let the system scroll what is not theirs", () => {
  assert.match(glue, /if \(!frame\) frame = requestAnimationFrame\(paint\);/);
  assert.match(glue, /swipe\.samples = keepSamples\(swipe\.samples, \{ t: e\.timeStamp, x: touch\.clientX, y: touch\.clientY \}\);/);
  assert.match(glue, /const speed = dragSpeed\(swipe\.samples, at\);/);
  assert.match(glue, /document\.addEventListener\("scroll", \(\) => \{\n\s+if \(swipe && !swipe\.axis\) swipe = null;\n\s+\}, \{ capture: true, passive: true \}\);/);
  assert.match(rule(' .app[data-pane="chat"] :is(.messages, .thread-messages)'), /touch-action: pan-y;/);
  assert.match(rule(" :is(.pt-hold, .pt-scrim)"), /touch-action: none;/);
  assert.match(rule(" :is(.pt-pick, .pt-pick-step, .ctx-menu, .composer-menu, .member-card > .mc-body, .pt-search-body)"), /overscroll-behavior: none;/);
});

test("a held message answers at once and opens its sheet in a third of a second", () => {
  assert.equal(LONG_PRESS.ms, 350);
  assert.equal(MOTION.pressShowMs, 80);
  for (const part of [".msg", ".channel", ".dm-thread", ".rail-item"]) assert.ok(PRESSABLE.split(", ").includes(part), part);
  assert.match(app, /show: setTimeout\(\(\) => pressed && pressed\.isConnected && pressed\.classList\.add\("pressing"\), MOTION\.pressShowMs\),/);
  assert.match(app, /if \(opened && navigator\.vibrate\) navigator\.vibrate\(8\);/);
  assert.match(rule(" .msg.pressing"), /background-color: var\(--bg-hover\);/);
  assert.match(css, /@media \(hover: none\) and \(pointer: coarse\) \{\n  html\[data-look="studio"\]\[data-fit="slim"\] :is\(\.msg, \.channel, \.dm-thread, \.rail-item, \.member, \.voice-user\) \{ -webkit-user-select: none; user-select: none; -webkit-touch-callout: none; \}/);
});

test("sheets rise from the bottom edge and fall back to it, and the held message stays where it is", () => {
  assert.match(css, /\.app\.pt-hold-open > \.pt-hold,\n[^\n]*\.app\.pt-pick-open > \.pt-pick \{ animation: pt-sheet-rise 320ms var\(--ease-settle\); \}/);
  assert.match(css, /@keyframes pt-sheet-rise \{\n  from \{ translate: 0 var\(--pt-sheet-h, 100%\); \}\n\}/);
  assert.match(css, /@keyframes pt-held-lift \{\n  from \{ translate: 0 calc\(-1 \* var\(--pt-sheet-h, 0px\)\); scale: \.96; opacity: \.4; \}\n\}/);
  assert.match(rule(" .app.pt-sheets-out > :is(.pt-hold, .pt-pick)"), /animation: pt-sheet-fall 200ms var\(--ease-out\) forwards; pointer-events: none;/);
  assert.equal(MOTION.sheetOutMs, 200);
  assert.match(glue, /const STUDIO_FALLING_SHEETS = \["pt-hold-open", "pt-pick-open"\];/);
  assert.match(glue, /studioPhone\.falling = \{ names: open, timer: setTimeout\(endSheetFall, MOTION\.sheetOutMs\) \};/);
  assert.match(glue, /return !studioPhone\.falling && STUDIO_SHEETS\.some\(name => app\.classList\.contains\(name\)\);/);
  assert.match(glue, /\$\("pt-hold"\)\.style\.setProperty\("--pt-sheet-h", `\$\{\$\("pt-hold"\)\.offsetHeight\}px`\);/);
  assert.match(glue, /if \(\["pt-hold", "pt-pick", "pt-search"\]\.includes\(sheet\.id\)\) closeSheets\(undefined, true\);/);
});

test("scrolling up through a chat brings the tab bar in, and scrolling down or reaching the end gives the message bar back", () => {
  const rest = { up: false, run: 0 };
  assert.deepEqual(chatBarAfter(rest, -10, 400), { up: false, run: -10 });
  assert.deepEqual(chatBarAfter({ up: false, run: -20 }, -8, 400), { up: true, run: 0 });
  assert.deepEqual(chatBarAfter({ up: false, run: -20 }, 5, 400), { up: false, run: 5 });
  assert.deepEqual(chatBarAfter({ up: true, run: 0 }, 6, 400), { up: true, run: 6 });
  assert.deepEqual(chatBarAfter({ up: true, run: 6 }, 8, 400), { up: false, run: 0 });
  assert.deepEqual(chatBarAfter({ up: true, run: 0 }, -300, MOTION.barEnd), { up: false, run: 0 });
  assert.deepEqual(chatBarAfter({ up: true, run: 0 }, -5, MOTION.barEnd + 1), { up: true, run: -5 });
  assert.ok(MOTION.barShow > MOTION.barHide);
  assert.match(glue, /if \(!delta \|\| performance\.now\(\) - touchedAt > MOTION\.barTouchMs\) return;\n\s+if \(!chatBarMayRise\(\)\) return lowerChatBar\(\);/);
  assert.match(glue, /studioPhone\.chatBar = chatBarAfter\(studioPhone\.chatBar, delta, log\.scrollHeight - log\.scrollTop - log\.clientHeight\);/);
  assert.match(glue, /&& document\.activeElement !== input && !input\.value\.trim\(\)\n\s+&& \$\("reply-bar"\)\.classList\.contains\("hidden"\)/);
  assert.match(glue, /\$\("message-input"\)\.addEventListener\("focus", lowerChatBar\);/);
  assert.match(glue, /if \(phonePane\(\) !== "chat" \|\| app\.classList\.contains\("thread-open"\)\) lowerChatBar\(\);/);
  const bar = rule(' .app[data-pane="chat"]:not(.thread-open) > .hzn');
  assert.match(bar, /display: block;\n  height: calc\(var\(--hz-line\) \+ 76px\);/);
  assert.match(bar, /visibility: hidden;\n  opacity: 0;\n  translate: 0 14px;/);
  assert.match(rule(' .app[data-pane="chat"].pt-bar-up:not(.thread-open) > .hzn'), /visibility: visible; opacity: 1; translate: 0 0;/);
  assert.match(rule(' .app[data-pane="chat"].pt-bar-up .main > :is(.composer, .typing-strip)'), /opacity: 0; translate: 0 14px; pointer-events: none;/);
  assert.match(rule('.keyboard-up .app[data-pane="chat"]:not(.thread-open) > .hzn'), /display: none;/);
});

test("a tab pressed from inside a chat goes back to the lists first, then opens that tab", () => {
  assert.match(glue, /if \(tab\) return openPhoneTab\(tab\.dataset\.tab\);/);
  assert.match(glue, /if \(phonePane\(\) === "list"\) return setPhoneTab\(tab\);\n\s+lowerChatBar\(\);\n\s+studioPhone\.tabNext = tab;/);
  assert.match(glue, /if \(studioPhone\.tabNext && phonePane\(\) === "list"\) \{\n\s+const tab = studioPhone\.tabNext;\n\s+studioPhone\.tabNext = null;\n\s+setPhoneTab\(tab\);/);
});

test("people who ask for less motion get none of it", () => {
  const calm = css.slice(css.indexOf("@media (prefers-reduced-motion: reduce)"));
  assert.match(calm, /\.app > :is\(\.main, \.rail, \.sidebar\) \{ animation: none !important; \}/);
  assert.match(code, /const calm = window\.matchMedia\("\(prefers-reduced-motion: reduce\)"\)\.matches;/);
  assert.match(glue, /const falls = !atOnce && !studioPhone\.calm && studioSlim\(\)/);
});

test("every sheet rises the same way, follows the finger and falls when it closes", () => {
  const rise = css.slice(css.indexOf(`${SLIM} .modal-backdrop > :is(.modal, .modal.dialog.slab),`));
  const head = rise.slice(0, rise.indexOf("}"));
  for (const part of [":is(.pop-menu.ctx-menu, .emoji-pop, .member-card)", ".app.pt-search-open > .pt-search", ".pt-room.opts ~ .pt-voice-sheet", ".pt-picker > .pt-picker-sheet"]) assert.ok(head.includes(`${SLIM} ${part}`), part);
  assert.match(head, /animation: pt-sheet-rise 320ms var\(--ease-settle\);/);
  assert.match(rule(" :is(.emoji-pop, .pt-voice-sheet, .modal-backdrop > .modal).pt-dragging"), /transition: none;/);
  const late = css.slice(css.indexOf("@supports (transition-behavior: allow-discrete) {"));
  assert.match(late, /\.modal-backdrop \{ transition: opacity 200ms var\(--ease-out\), display 200ms allow-discrete; \}/);
  assert.match(late, /:is\(\.pop-menu\.ctx-menu, \.emoji-pop, \.member-card\)\.hidden \{ translate: 0 calc\(100% \+ 48px\); pointer-events: none;/);
  assert.match(late, /\.pt-room:not\(\.opts\) ~ \.pt-voice-sheet \{ translate: 0 calc\(100% \+ 48px\); pointer-events: none;/);
  assert.match(glue, /const STUDIO_SHEET_NODES = "#pt-hold, #pt-pick, #pt-search, #pt-voice-sheet, #emoji-pop, \.member-card, \.ctx-menu, \.composer-menu, \.audio-pop, \.soundboard-pop, #modal-accounts";/);
  assert.match(glue, /else if \(sheet\.id === "pt-voice-sheet"\) closeVoiceOptions\(\);/);
});

test("Settings and Events push in from the right and go back with a swipe from the left edge", () => {
  assert.match(rule(" :is(.settings-layer, #events-layer)"), /animation: pt-push-in 300ms var\(--ease-settle\);/);
  assert.match(rule(" .settings-layer > .settings-content"), /animation: pt-push-in 300ms var\(--ease-settle\);/);
  assert.match(rule(" :is(.settings-layer, #events-layer, .settings-layer > .settings-content).pt-dragging"), /transition: none;/);
  assert.match(rule(" .settings-layer.pt-page-back > .settings-content"), /z-index: 2; display: flex;/);
  assert.match(css, /:is\(\.settings-layer, #events-layer\)\.hidden \{ translate: 100% 0; pointer-events: none; transition: translate 240ms var\(--ease-out\), display 240ms allow-discrete; \}/);
  assert.match(glue, /const STUDIO_PAGE_LAYERS = \["user-settings", "server-settings", "channel-settings", "events-layer"\];/);
  assert.match(glue, /if \(floatingOpen\(\)\) return null;\n  const layer = openPageLayer\(\);\n  if \(layer\) return edgeBackStarts\(touch\.clientX\) && !sheetOpen\(\) \? \{ kind: "back", \.\.\.pageLayerBack\(layer\) \} : null;/);
  assert.match(glue, /if \(layer\.id === "events-layer"\) return \{ node: layer, then: \(\) => \$\("events-close"\)\.click\(\) \};/);
  assert.match(glue, /return \{ node: layer, then: \(\) => layer\.querySelector\("\.settings-phone-close"\)\.click\(\) \};/);
  assert.match(glue, /if \(swipe\.then\) return slideAway\(node, left, speed, swipe\.then\);/);
  assert.match(glue, /if \(swipe\.begin\) swipe\.begin\(\);/);
  assert.match(glue, /if \(swipe\.stay\) swipe\.stay\(\);/);
});

test("a settings page shows its menu underneath while it is dragged back, and returns if let go early", () => {
  assert.match(glue, /begin: \(\) => \{\n      clearTimeout\(studioPhone\.pageBackTimer\);\n      layer\.classList\.add\("pt-page-back"\);\n      settingsPhoneView\(layer, "menu"\);\n    \},/);
  assert.match(glue, /stay: \(\) => \{\n      settingsPhoneView\(layer, "page"\);/);
  assert.match(glue, /then: \(\) => \{\n      settle\(\);\n      if \(history\.state && history\.state\.page\) history\.back\(\);\n    \},/);
});

test("a pinch keeps the point under the fingers where it is", () => {
  const rest = { scale: 1, x: 0, y: 0 };
  const grip = gripOf(rest, 40, -20);
  assert.deepEqual(grip, { x: 40, y: -20 });
  assert.deepEqual(viewHolding(grip, 2, 40, -20), { scale: 2, x: -40, y: 20 });
  assert.deepEqual(viewHolding(grip, 2, 60, 0), { scale: 2, x: -20, y: 40 });
  const zoomed = { scale: 2, x: -40, y: 20 };
  assert.deepEqual(gripOf(zoomed, 40, -20), { x: 40, y: -20 });
  assert.deepEqual(viewHolding(gripOf(zoomed, 0, 0), 1, 0, 0), { scale: 1, x: -20, y: 10 });
  assert.equal(pinchScale(1, 2, 4), 2);
  assert.equal(pinchScale(2, 3, 4), 4);
  assert.equal(pinchScale(1, 0.4, 4), 1);
});

test("a zoomed picture cannot be dragged past its own edge, and a small one stays put", () => {
  assert.equal(panWithin(50, 100, 195, 390), 0);
  assert.equal(panWithin(0, 400, 195, 390), 0);
  assert.equal(panWithin(900, 400, 195, 390), 205);
  assert.equal(panWithin(-900, 400, 195, 390), -205);
  assert.equal(panWithin(120, 400, 195, 390), 120);
});

test("the viewer dims as the picture is pulled away, never past forty percent", () => {
  assert.equal(pullFade(0, 0, 420), 1);
  assert.equal(pullFade(0, 210, 420), 0.5);
  assert.equal(pullFade(0, 2000, 420), 0.4);
  assert.equal(pullFade(0, -300, 420), 1);
  assert.equal(pullFade(105, 0, 420), 0.75);
});

test("the picture viewer takes a pinch, a double tap, a pull down and an edge swipe", () => {
  assert.match(viewer, /const VIEWER = \{ most: 4, tap: 2\.5, doubleMs: 280, doubleSlop: 30, slop: 8, awayAt: 110, fadeOver: 420, settleMs: 260, awayMs: 200 \};/);
  assert.match(viewer, /const down = !cancelled && sheetSettles\(drag\.dy, speed\.y, VIEWER\.awayAt\);/);
  assert.match(viewer, /const back = !cancelled && !down && drag\.edge && backSettles\(drag\.dx, speed\.x, window\.innerWidth\);/);
  assert.match(viewer, /if \(viewerTouch\.view\.scale === 1\) viewerTouch\.tapTimer = setTimeout\(closeLightbox, VIEWER\.doubleMs\);/);
  assert.match(viewer, /viewerPaint\(viewerHeld\(viewHolding\(gripOf\(view, fx, fy\), VIEWER\.tap, fx, fy\), rest\), VIEWER\.settleMs\);/);
  assert.match(viewer, /if \(!studioSlim\(\) \|\| e\.touches\.length > 2 \|\| e\.target\.closest\("\.lightbox-actions"\)\) return;/);
  assert.match(viewer, /box\.addEventListener\("touchmove", viewerTouchMove, \{ passive: false \}\);/);
  assert.match(viewer, /if \(shown === phoneViewer\.shown\) return;\n    phoneViewer\.shown = shown;\n    viewerReset\(\);/);
  assert.match(rule(" #lightbox"), /touch-action: none;/);
  assert.match(rule(" #lightbox:not(.hidden)"), /animation: pt-scrim-in 200ms var\(--ease-out\);/);
  assert.match(css, /#lightbox\.hidden > #lightbox-img \{ visibility: hidden; \}/);
  assert.match(glue, /const STUDIO_FLOATING = "\.modal-backdrop:not\(\.hidden\), \.pop-menu:not\(\.hidden\), #emoji-pop:not\(\.hidden\), #member-card:not\(\.hidden\), #lightbox:not\(\.hidden\)";/);
});
