import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { STUDIO_DIALOGS, dialogAnswerClasses } = require("../scripts/studio-dialogs.js");
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
