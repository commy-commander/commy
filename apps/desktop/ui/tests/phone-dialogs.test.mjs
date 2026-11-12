import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { PHONE_DIALOG_HEADS, PHONE_DIALOG_SAFE, phoneDialogGate, phoneDialogSafeId } = require("../scripts/phone-dialogs.js");
const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-dialogs.css");
const glue = read("scripts", "phone-dialogs.js");
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';
const S = `${SLIM} .modal.dialog.slab`;

const rule = selector => {
  const at = css.indexOf(`${selector} {`);
  assert.notEqual(at, -1, selector);
  return css.slice(at, css.indexOf("}", at));
};

test("every phone dialog rule is for the phone app in the new look, with tokens and no comments", () => {
  const selectors = css.split("}").map(block => block.split("{")[0]).flatMap(head => head.split(",\n")).map(s => s.trim()).filter(Boolean);
  assert.ok(selectors.length > 40);
  for (const selector of selectors) assert.ok(selector.startsWith(SLIM), selector);
  assert.equal(/#[0-9a-fA-F]{3,8}\b/.test(css.replace(/#(modal|signin|invite)-[a-z-]+/g, "")), false);
  assert.equal(css.includes("/*"), false);
  assert.equal(/\/\/|\/\*/.test(glue.replace(/\/\^[^/]*\//g, "")), false);
});

test("the phone dialog files sit with the other phone files, before the off look", () => {
  assert.match(html, /id="studio-phone" href="styles\/phone-touch\.css" media="not all" \/>\n(\s*<link rel="stylesheet" id="studio-phone-[a-z-]+" href="styles\/phone-[a-z-]+\.css" media="not all" \/>\n)*\s*<link rel="stylesheet" id="studio-phone-dialogs" href="styles\/phone-dialogs\.css" media="not all" \/>\n(\s*<link rel="stylesheet" id="studio-phone-[a-z-]+" href="styles\/phone-[a-z-]+\.css" media="not all" \/>\n)*\s*<link rel="stylesheet" id="studio-phone-off" href="styles\/phone-off\.css" media="not all" \/>\n(?!\s*<link rel="stylesheet" id="studio-phone)/);
  assert.match(html, /<script src="scripts\/studio-phone\.js"><\/script>\n(\s*<script src="scripts\/phone-[a-z-]+\.js"><\/script>\n)*\s*<script src="scripts\/phone-dialogs\.js"><\/script>/);
});

test("a dialog on the phone is a bottom sheet as tall as its content, stopping under the status bar", () => {
  assert.match(rule(`${SLIM} .modal-backdrop:has(> .modal.dialog:not(.hidden))`), /align-items: flex-end;.*padding: 0 0 var\(--keyboard-height, 0px\);\n  background: var\(--scrim\);/s);
  const sheet = rule(S);
  assert.match(sheet, /width: 100% !important;/);
  assert.match(sheet, /max-height: calc\(var\(--app-height, 100dvh\) - env\(safe-area-inset-top\) - 56px\) !important;/);
  assert.match(sheet, /padding: 30px 18px calc\(40px \+ env\(safe-area-inset-bottom\)\);/);
  assert.match(sheet, /overflow: hidden auto;\n  overscroll-behavior: contain;/);
  assert.match(sheet, /border-radius: var\(--radius-sheet\) var\(--radius-sheet\) 0 0 !important;/);
  assert.match(sheet, /var\(--sheet-bg\);/);
  assert.match(rule(`${S} > *`), /flex-shrink: 0;/);
});

test("the grab bar is the sheets' 38 by 5 bar, hidden on gates", () => {
  const grab = rule(`${S}::after`);
  assert.match(grab, /position: sticky;/);
  assert.match(grab, /width: 38px;\n  height: 5px;/);
  assert.match(grab, /border-radius: 3px;\n  background: var\(--line-strong\);/);
  assert.match(rule(`${S}.pd-gate::after`), /visibility: hidden;/);
});

test("the title stays pinned at the top and the answers at the bottom while the middle scrolls", () => {
  assert.match(rule(`${S} > h2`), /font: 700 22px\/1\.2 var\(--font-display\);/);
  assert.match(rule(`${S} > .pd-head`), /position: sticky;\n  top: -30px;/);
  const answers = rule(`${S} > .modal-actions.answers`);
  assert.match(answers, /position: sticky;\n  bottom: 0;/);
  assert.match(answers, /grid-template-columns: minmax\(0, 1fr\) minmax\(0, 1fr\);\n  gap: 10px;/);
  assert.match(answers, /padding: 0 18px calc\(40px \+ env\(safe-area-inset-bottom\)\);/);
  assert.match(rule(`${S}.pd-scrolls > .modal-actions.answers`), /background: linear-gradient\(var\(--sheet-bg\), var\(--sheet-bg\)\), var\(--sheet-bg\);/);
  assert.deepEqual(PHONE_DIALOG_HEADS, ["h2", ".sw-head", ".notify-head", ".catalog-head"]);
  assert.match(glue, /modal\.classList\.toggle\("pd-scrolls", modal\.scrollHeight > modal\.clientHeight \+ 1\);/);
});

test("answers are one row of equal 52 high keys, one alone runs full width, a third sits full width above", () => {
  assert.match(rule(`${S} > .answers > .key`), /width: 100%;.*height: 52px;.*font: 600 16px\/1\.15 var\(--font\);/s);
  assert.match(rule(`${S} > .answers:not(:has(> button:not(.hidden) ~ button:not(.hidden))) > .key`), /grid-column: 1 \/ -1;/);
  const three = `${S} > .answers:has(> button:not(.hidden) ~ button:not(.hidden) ~ button:not(.hidden))`;
  assert.ok(css.includes(`${three} > .key.third,\n${three} > .key.out-k,\n${three}:not(:has(> .third:not(.hidden), > .out-k:not(.hidden))) > .key:first-child {\n  grid-column: 1 / -1;\n  order: -1;\n}`));
});

test("yes or no rows use the phone's switch, with the Day off track", () => {
  assert.match(rule(`${S} input.switch`), /width: 54px;\n  height: 32px;.*background: var\(--tog-off\);/s);
  assert.match(rule(`${S} input.switch::after`), /width: 26px;\n  height: 26px;/);
  assert.match(rule(`${S} input.switch:checked::after`), /translate: 22px 0;\n  background: var\(--cream\);/);
  assert.match(rule(`${SLIM}[data-theme="light"] .modal.dialog.slab input.switch:not(:checked)`), /background: var\(--bg-active\);\n  box-shadow: inset 0 1px 2px rgb\(var\(--dark\) \/ \.45\), 0 1px 0 rgb\(var\(--lamp\) \/ \.5\);/);
});

test("pick lists are pick rows, not radio circles or check boxes", () => {
  assert.match(rule(`${SLIM} #modal-choose-roles.dialog .role-pick,\n${S} > .report-reasons > label`), /height: 46px;.*border-radius: var\(--radius-pill\);/s);
  assert.match(rule(`${SLIM} #modal-choose-roles.dialog .role-pick.held,\n${S} > .report-reasons > label:has(> input:checked)`), /box-shadow: inset 0 0 0 1\.5px var\(--accent-lit\);/);
  assert.match(rule(`${SLIM} #modal-choose-roles.dialog .role-pick > .role-pick-check`), /border: 0;.*background: none;.*opacity: 0;/s);
});

test("gates close only from their own answers", () => {
  for (const id of ["modal-username", "modal-claim", "modal-call"]) assert.equal(phoneDialogGate(id), true, id);
  assert.equal(phoneDialogGate("modal-signin", { signedIn: false }), true);
  assert.equal(phoneDialogGate("modal-signin", { signedIn: true }), false);
  assert.equal(phoneDialogGate("modal-server-welcome", { laterShown: false }), true);
  assert.equal(phoneDialogGate("modal-server-welcome", { laterShown: true }), false);
  assert.equal(phoneDialogGate("modal-rules", { acceptShown: true }), true);
  assert.equal(phoneDialogGate("modal-rules", { acceptShown: false }), false);
  for (const id of ["modal-leave-server", "modal-server-notify", "modal-create-channel", "modal-onboarding"]) assert.equal(phoneDialogGate(id), false, id);
  assert.match(glue, /if \(modal\.classList\.contains\("pd-gate"\)\) \{\n    history\.pushState/);
  assert.match(glue, /if \(e\.type === "mousedown" && !modal\.classList\.contains\("pd-gate"\)\) phoneDialogSafe\(modal\);/);
  assert.match(glue, /modal\.classList\.contains\("pd-gate"\) \|\| phoneDialogIsGate\(modal\) \|\| !modal\.contains\(e\.target\)\) return;/);
});

test("back, swipe down and a scrim tap pick the safe answer", () => {
  assert.equal(PHONE_DIALOG_SAFE.length, 7);
  assert.equal(phoneDialogSafeId(["leave-server-cancel", "leave-server-confirm"]), "leave-server-cancel");
  assert.equal(phoneDialogSafeId(["ob-later", "ob-back", "ob-next"]), "ob-later");
  assert.equal(phoneDialogSafeId(["call-decline", "call-accept"]), null);
  assert.equal(phoneDialogSafeId(["join-wait-cancel", "join-wait-send", "join-wait-close"]), "join-wait-close");
  assert.equal(phoneDialogSafeId(["create-server-back", "create-server-open"]), "create-server-back");
  assert.equal(phoneDialogSafeId(["server-notify-done"]), "server-notify-done");
  assert.equal(phoneDialogSafeId(["stream-deck-deny", "stream-deck-allow"]), "stream-deck-deny");
  assert.equal(phoneDialogSafeId(["poll-create"]), null);
  assert.match(glue, /const step = layerStep\("ptDialog", open, phoneDialogs\.held, history\.state\);/);
  assert.match(glue, /swipe\.dy = sheetPull\(dy\);/);
  assert.match(glue, /const speed = dragSpeed\(swipe\.samples, at\)\.y;/);
  assert.match(glue, /if \(cancelled \|\| !sheetSettles\(swipe\.dy, speed, SWIPE\.sheetAt\)\) \{/);
  assert.match(glue, /const ms = calm \? 0 : settleMs\(modal\.offsetHeight - swipe\.dy, speed, MOTION\.sheetOutMs, 120\);/);
  assert.match(glue, /return backdrop\.querySelector\(":scope > \.modal:not\(\.hidden\)"\);/);
  assert.match(glue, /window\.addEventListener\("popstate", phoneDialogOnBack\);/);
});
