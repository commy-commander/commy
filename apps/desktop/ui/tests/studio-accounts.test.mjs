import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio-accounts.css");
const app = read("../scripts/app.js");
const glue = read("../scripts/studio-accounts.js");

const LOOK = 'html[data-look="studio"]';
const SLIM = 'html[data-look="studio"][data-fit="slim"]';
const M = "#modal-accounts";
const selectors = [...css.matchAll(/(^|\})\s*([^{}@]+)\{/g)].flatMap(match => match[2].split(",").map(part => part.trim()).filter(Boolean));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("the Accounts window's new look stays off until the switch is on", () => {
  assert.ok(selectors.length > 30);
  for (const selector of selectors) assert.ok(selector.startsWith('html[data-look="studio"]'), selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  const sheet = html.indexOf('<link rel="stylesheet" id="studio-accounts" href="styles/studio-accounts.css" media="not all" />');
  assert.ok(sheet > html.indexOf('id="studio-settings"'));
  assert.ok(sheet < html.indexOf('id="studio-phone"'));
  const script = html.indexOf('<script src="scripts/studio-accounts.js"></script>');
  assert.ok(script > html.indexOf('<script src="scripts/studio-settings.js"></script>'));
  assert.ok(script < html.indexOf('<script src="scripts/app.js"></script>'));
  assert.match(app, /if \(studioLook\(\)\) studioSettingsShell\(closeSettingsLayers\);\n  if \(studioLook\(\)\) studioAccountsWindow\(\);/);
  assert.match(glue, /modal\.classList\.add\("slab"\);/);
  assert.match(glue, /if \(add\) add\.classList\.add\("key"\);/);
});

test("the desktop window is the guide's dialog slab with 60 px capsule rows", () => {
  assert.match(rule(`${LOOK} .modal.slab${M}`), /width: min\(440px, calc\(100vw - 32px\)\);\n  padding: 26px 28px 24px;/);
  assert.match(rule(`${LOOK} ${M} > h2`), /font: 700 21px\/1\.2 var\(--font-display\);/);
  const row = rule(`${LOOK} ${M} .account-row`);
  for (const line of ["gap: 12px;", "height: 60px;", "border-radius: var(--radius-pill);"]) assert.ok(row.includes(line), line);
  assert.match(rule(`${LOOK} ${M} .account-row.current`), /outline: var\(--ring-on\);\n  outline-offset: -2px;/);
  assert.match(rule(`${LOOK} ${M} .account-row:not(.current) .account-avatar`), /background: var\(--bg-active\);/);
  assert.match(rule(`${LOOK} ${M} .account-sign-out`), /color: var\(--danger-text\);/);
  assert.match(rule(`${LOOK} ${M} #account-add.key`), /height: 48px;\n  margin-top: 14px;/);
  assert.match(rule(`${LOOK} ${M} .account-note.error`), /color: var\(--danger-text\);/);
  assert.ok(css.includes(`html[data-look="studio"]:not([data-theme="light"]) ${M} #account-add.key:disabled {`));
});

test("on the phone the window is a bottom sheet with a grab bar", () => {
  assert.match(rule(`${SLIM} .modal-backdrop:has(> ${M}:not(.hidden))`), /align-items: flex-end;/);
  assert.match(rule(`${SLIM} .modal.slab${M}`), /border-radius: var\(--radius-sheet\) var\(--radius-sheet\) 0 0;/);
  assert.match(rule(`${SLIM} ${M} > .set-grab`), /width: 40px;\n  height: 5px;/);
  assert.match(rule(`${SLIM} ${M} #account-add.key`), /height: 50px;/);
  assert.match(rule(`${SLIM} ${M} .account-name`), /font-size: 16px;/);
});
