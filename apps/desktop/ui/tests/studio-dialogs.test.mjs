import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { STUDIO_DIALOGS, DIALOG_BLUE_ACTIONS, dialogAnswerClasses, JOIN_DIALOGS, JOIN_ANSWERS, joinAnswerClasses, joinTileServer, dialogJustOpened, MODERATE_DIALOGS, leavingTitleIcon } = require("../scripts/studio-dialogs.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio-dialogs.css");
const app = read("../scripts/app.js");
const glue = read("../scripts/studio-dialogs.js");

const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const selectors = [...css.matchAll(/(^|\})\s*([^{}@]+)\{/g)].flatMap(match => match[2].split(/,\n/).map(part => part.trim()).filter(Boolean));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("the dialogs' new look stays off until the switch is on and off on the phone", () => {
  assert.ok(selectors.length > 20);
  for (const selector of selectors) assert.ok(selector.startsWith(R) || selector.startsWith('html[data-look="studio"][data-fit="slim"]'), selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  const sheet = html.indexOf('<link rel="stylesheet" id="studio-dialogs" href="styles/studio-dialogs.css" media="not all" />');
  assert.ok(sheet > html.indexOf('id="studio-profile-editor"'));
  assert.ok(sheet < html.indexOf('id="studio-phone"'));
  const script = html.indexOf('<script src="scripts/studio-dialogs.js"></script>');
  assert.ok(script > html.indexOf('<script src="scripts/studio-accounts.js"></script>'));
  assert.ok(script < html.indexOf('<script src="scripts/app.js"></script>'));
  assert.match(app, /\n  if \(studioLook\(\)\) studioDialogs\(\);\n/);
  assert.ok(glue.includes('const roomy = document.documentElement.dataset.fit !== "slim";'));
  assert.ok(glue.includes('attributeFilter: ["data-fit"]'));
});

test("step one covers the plain confirms and small forms, each with its answers row", () => {
  assert.equal(STUDIO_DIALOGS.length, 21);
  for (const id of STUDIO_DIALOGS) {
    const start = html.indexOf(`id="${id}"`);
    assert.ok(start > 0, id);
    const rest = html.slice(start + 1);
    const found = rest.search(/<div class="modal[ "]/);
    const next = found >= 0 ? start + 1 + found : -1;
    assert.ok(html.slice(start, next > 0 ? next : undefined).includes('<div class="modal-actions">'), id);
  }
  for (const id of ["modal-accounts", "modal-profile", "modal-share", "modal-rules", "modal-signin"]) assert.equal(STUDIO_DIALOGS.includes(id), false, id);
});

test("a dialog gets one bright key: cream beside red, plain glass beside blue", () => {
  assert.deepEqual(dialogAnswerClasses(["plain", "danger"]), ["key cream", "key danger"]);
  assert.deepEqual(dialogAnswerClasses(["plain", "primary"]), ["key", "key primary"]);
  assert.deepEqual(dialogAnswerClasses(["plain", "plain", "primary"]), ["key", "key third", "key primary"]);
  assert.deepEqual(dialogAnswerClasses(["primary"]), ["key primary"]);
  assert.deepEqual(dialogAnswerClasses(["plain"]), ["key"]);
  for (const classes of [["plain", "danger"], ["plain", "primary"], ["plain", "plain", "primary"]]) {
    const keys = dialogAnswerClasses(classes);
    assert.equal(keys.filter(key => /cream|primary|danger/.test(key)).length <= 2, true);
    assert.equal(keys.some(key => key.includes("cream")) && keys.some(key => key.includes("primary")), false);
  }
  assert.match(rule(`${R} .modal.dialog > .answers > .key.third`), /order: -1;/);
});

test("the dialog is the guide's slab with its answers on the bottom edge", () => {
  assert.match(rule(`${R} .modal.dialog`), /width: min\(400px, calc\(100vw - 48px\)\);\n  padding: 26px 28px 48px;\n  border: 0;\n  border-radius: var\(--radius-xl\);/);
  assert.match(rule(`${R} .modal.dialog > h2`), /font: 700 21px\/1\.2 var\(--font-display\);/);
  assert.match(rule(`${R} .modal.dialog > .modal-actions.answers`), /position: absolute;\n  right: 24px;\n  bottom: -20px;/);
  assert.match(rule(`${R} .modal-backdrop:has(> .modal.dialog:not(.hidden))`), /background: var\(--scrim\);/);
  assert.match(rule(`${R} .modal.dialog > .answers > .key:disabled`), /opacity: 1;/);
});

test("the four undrawn parts follow the guide's reading", () => {
  assert.ok(glue.includes('studioIconMarkup("triangle-alert")'));
  assert.ok(glue.includes('studioIconMarkup("check", "tick")'));
  assert.match(rule(`${R} .modal.dialog > .danger-callout`), /border: 0;\n  border-radius: 14px;\n  background: var\(--input-bg\);\n  box-shadow: var\(--slot\);\n  font: 400 14px\/1\.4 var\(--font\);/);
  assert.match(rule(`${R} .modal.dialog > .danger-callout > .i`), /color: var\(--danger-text\);/);
  assert.match(rule(`${R} .modal.dialog > .tray-opt + .tray-opt`), /box-shadow: inset 0 1px 0 var\(--line\);/);
  assert.match(rule(`${R} .modal.dialog > .tray-opt`), /background: none;/);
  assert.match(rule(`${R} .modal.dialog > .report-reasons > label`), /height: 32px;\n  padding: 0 10px;\n  border-radius: 7px;/);
  assert.match(rule(`${R} .modal.dialog > .report-reasons > label > input`), /opacity: 0;/);
  assert.ok(css.includes(`${R} .modal.dialog > .report-reasons > label:has(> input:checked) > .tick { opacity: 1; }`));
  assert.match(rule(`${R} .modal.dialog > .delete-preview`), /border-radius: 14px;\n  background: var\(--input-bg\);\n  box-shadow: var\(--slot\);/);
  assert.match(rule(`${R} .modal.dialog > .delete-preview > .avatar`), /width: 32px;\n  height: 32px;/);
  assert.ok(css.includes('html[data-look="studio"][data-fit="slim"] .danger-callout > .i,\nhtml[data-look="studio"][data-fit="slim"] .report-reasons > label > .tick { display: none; }'));
});

test("reporting is a blue action with a switch row to also block", () => {
  assert.deepEqual(DIALOG_BLUE_ACTIONS, ["report-go"]);
  assert.ok(glue.includes('if (DIALOG_BLUE_ACTIONS.includes(button.id)) return "primary";'));
  assert.ok(glue.includes(`box.classList.toggle("switch", roomy)`));
  assert.match(rule(`${R} .modal.dialog > .report-also-block`), /min-height: 44px;/);
  assert.match(rule(`${R} .modal.dialog > .report-reasons > label:has(> input:checked)`), /background: var\(--bg-active\);/);
  assert.ok(css.includes("resize: none;"));
});

const J = `${R} .modal.dialog.join`;

test("joining a server: six dialogs, each answer named, red words on the left", () => {
  assert.deepEqual(JOIN_DIALOGS, ["modal-invite-link", "modal-join-wait", "modal-server-welcome", "modal-onboarding", "modal-rules", "modal-server-info"]);
  for (const id of JOIN_DIALOGS) assert.ok(html.includes(`id="${id}"`), id);
  for (const id of Object.keys(JOIN_ANSWERS)) assert.ok(html.includes(`id="${id}"`), id);
  assert.deepEqual(joinAnswerClasses(["join-wait-cancel", "join-wait-send", "join-wait-close"]), ["key out-k", "key", "key primary"]);
  assert.deepEqual(joinAnswerClasses(["si-leave", "si-answers", "si-close"]), ["key out-k", "key", "key primary"]);
  assert.deepEqual(joinAnswerClasses(["invite-link-cancel", "invite-link-join"]), ["key", "key primary"]);
  for (const id of Object.keys(JOIN_ANSWERS)) assert.ok(JOIN_ANSWERS[id].split(" ").filter(name => name === "primary").length <= 1, id);
  assert.match(rule(`${J} > .modal-actions.answers.spread`), /left: 24px;/);
  assert.match(rule(`${J} > .answers > .key.out-k`), /margin-right: auto;\n  color: var\(--danger-text\);/);
  assert.match(rule(`${J}`), /width: min\(440px, calc\(100vw - 48px\)\);/);
  assert.match(rule(`${R} .modal.modal-wide.dialog.join`), /width: min\(560px, calc\(100vw - 48px\)\);/);
});

test("the head row's tile takes the strip's colour, with no id before you join", () => {
  assert.equal(joinTileServer("modal-invite-link", "s1", "w1"), null);
  assert.equal(joinTileServer("modal-join-wait", "s1", "w1"), "w1");
  assert.equal(joinTileServer("modal-server-info", "s1", "w1"), "s1");
  assert.equal(joinTileServer("modal-rules", null, null), null);
  assert.match(rule(`${J} .sw-icon`), /width: 48px;\n  height: 48px;/);
  assert.ok(rule(`${J} .sw-icon::before`).includes("color-mix(in srgb, var(--server-tint, var(--accent)) 26%, var(--bg-raised))"));
  assert.ok(css.includes(`${J}#modal-join-wait:has(#join-wait-send:not(.hidden)) #join-wait-icon::after {`));
  assert.ok(glue.includes('icon.style.setProperty("--server-tint", serverTint(id));'));
});

test("hero, warning slot, rules slot and steps follow the joining drawing", () => {
  assert.ok(css.includes(`${J}#modal-invite-link.has-hero > .invite-link-hero,\n${J} > .si-banner:not(.hidden) {\n  position: relative;\n  display: block;\n  height: 112px;`));
  assert.ok(css.includes("margin-top: -48px;"));
  assert.ok(css.includes("box-shadow: 0 0 0 4px var(--bg-dark), var(--pillow-sm);"));
  assert.match(rule(`${J} > .invite-link-warning > .i`), /color: var\(--danger-text\);/);
  assert.ok(glue.includes('warning.insertAdjacentHTML("afterbegin", studioIconMarkup("triangle-alert"))'));
  assert.ok(css.includes(`${J} .join-wait-rules-text,\n${J} #si-rules { max-height: 132px; }`));
  assert.match(rule(`${J} #sw-steps > .sw-step.active`), /outline: var\(--ring-on\);/);
  assert.match(rule(`${J} #sw-steps > .sw-step.active > b`), /background: var\(--accent\);/);
  assert.match(rule(`${J} #sw-steps > .sw-step.done > .i`), /display: block;/);
  assert.match(rule(`${J} #ob-steps > .sw-step.active`), /width: 20px;/);
  assert.match(rule(`${J} #ob-steps`), /order: 1;/);
});

test("roles and answers are pick rows, banner roles grow into cards", () => {
  assert.ok(css.includes(`${J} .ob-answer,\n${J} .role-pick {\n  display: flex;\n  align-items: center;\n  gap: 12px;\n  width: 100%;\n  height: 46px;`));
  assert.ok(css.includes(`${J} .ob-answer.picked,\n${J} .role-pick.held {\n  background: var(--bg-active);\n  box-shadow: inset 0 0 0 1.5px var(--accent-lit);`));
  assert.match(rule(`${J} .role-pick.has-banner`), /border-radius: 18px;/);
  assert.match(rule(`${J} .role-pick.has-banner > .role-pick-banner`), /height: 38px;/);
  assert.ok(css.includes(`${J} .ob-check,\n${J} .role-pick > .role-pick-check {\n  order: 1;`));
  assert.ok(css.includes('html[data-look="studio"][data-fit="slim"] .invite-link-warning > .i,'));
});

test("a dialog opens without the focus ring until a key is pressed", () => {
  assert.equal(dialogJustOpened("modal hidden", false), true);
  assert.equal(dialogJustOpened("modal dialog slab", false), false);
  assert.equal(dialogJustOpened("modal hidden", true), false);
  assert.equal(dialogJustOpened(null, false), false);
  assert.ok(css.includes(`${R} .modal.dialog.quiet > .answers > .key:focus-visible { outline: 0; }`));
  assert.ok(glue.includes("attributeOldValue: true"));
  assert.ok(glue.includes('modal.classList.remove("quiet")'));
});

const D = `${R} .modal.dialog`;

test("moderating: who slot, latch keys for lengths, cream way out beside red", () => {
  assert.deepEqual(MODERATE_DIALOGS, ["modal-moderate", "modal-timeout", "modal-leaving"]);
  assert.ok(css.includes(`${D}.modal-moderate,\n${D}.modal-leaving { width: min(440px, calc(100vw - 48px)); }`));
  for (const id of MODERATE_DIALOGS) assert.ok(html.includes(`id="${id}"`), id);
  assert.match(rule(`${D} > .mod-who`), /padding: 10px 14px;\n  border-radius: var\(--radius-pill\);\n  background: var\(--input-bg\);\n  box-shadow: var\(--slot\);/);
  assert.match(rule(`${D} > .mod-who > .avatar`), /width: 32px;\n  height: 32px;/);
  assert.match(rule(`${D} > .timeout-choices`), /grid-template-columns: repeat\(3, minmax\(0, 1fr\)\);\n  gap: 8px;/);
  assert.match(rule(`${D} > .timeout-choices > .key.latch`), /height: 40px;/);
  assert.ok(glue.includes('const down = roomy && choice.classList.contains("active");'));
  assert.ok(glue.includes("keys.observe(grid, { childList: true })"));
  assert.deepEqual(dialogAnswerClasses(["plain", "danger"]), ["key cream", "key danger"]);
});

test("leaving Commy: icon in the title, address capsule, red dots, trust switch", () => {
  assert.equal(leavingTitleIcon(["modal", "modal-leaving"]), null);
  assert.equal(leavingTitleIcon(["modal", "modal-leaving", "warn"]), "triangle-alert");
  assert.equal(leavingTitleIcon(["modal", "modal-leaving", "block"]), "ban");
  assert.match(rule(`${D}.modal-leaving > h2 > .i`), /color: var\(--danger-text\);/);
  assert.match(rule(`${D} > .leaving-host`), /border-radius: var\(--radius-pill\);/);
  assert.match(rule(`${D} > .leaving-host`), /font: 400 15px\/1\.2 var\(--font-mono\);/);
  assert.match(rule(`${D} > .leaving-spelled`), /font: 400 12\.5px\/1\.3 var\(--font-mono\);/);
  assert.match(rule(`${D} > .leaving-notes > li::before`), /background: var\(--danger-text\);/);
  assert.match(rule(`${D} > .leaving-notes`), /list-style: none;/);
  assert.ok(css.includes(`${D}.modal-leaving { border: 0; }`));
  assert.match(rule(`${D} > .leaving-trust`), /min-height: 44px;/);
  assert.ok(glue.includes(":scope > .leaving-trust > input[type='checkbox']"));
  assert.match(rule(`${D} input.switch:checked`), /background: var\(--accent\);/);
});
