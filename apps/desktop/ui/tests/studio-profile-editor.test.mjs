import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { PROFILE_EDITOR_ORDER, presenceDotClass, aboutCount } = require("../scripts/studio-profile-editor.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio-profile-editor.css");
const app = read("../scripts/app.js");
const glue = read("../scripts/studio-profile-editor.js");

const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const SLIM = 'html[data-look="studio"][data-fit="slim"]';
const selectors = [...css.matchAll(/(^|\})\s*([^{}@]+)\{/g)].flatMap(match => match[2].split(",").map(part => part.trim()).filter(Boolean));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("the profile editor's new look stays off until the switch is on", () => {
  assert.ok(selectors.length > 50);
  for (const selector of selectors) assert.ok(selector.startsWith('html[data-look="studio"]'), selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  const sheet = html.indexOf('<link rel="stylesheet" id="studio-profile-editor" href="styles/studio-profile-editor.css" media="not all" />');
  assert.ok(sheet > html.indexOf('id="studio-accounts"'));
  assert.ok(sheet < html.indexOf('id="studio-phone"'));
  const script = html.indexOf('<script src="scripts/studio-profile-editor.js"></script>');
  assert.ok(script > html.indexOf('<script src="scripts/studio-accounts.js"></script>'));
  assert.ok(script < html.indexOf('<script src="scripts/app.js"></script>'));
  assert.match(app, /if \(studioLook\(\)\) studioAccountsWindow\(\);\n  if \(studioLook\(\)\) studioProfileEditor\(\);/);
  assert.match(app, /\$\("account-banner-remove"\)\.classList\.toggle\("hidden", !state\.user\.banner\);\n  if \(studioLook\(\)\) studioEditorBanner\(strip\);/);
});

test("the fields follow the ruled order and each one still exists", () => {
  assert.deepEqual(PROFILE_EDITOR_ORDER, ["account-display-name", "account-pronouns", "account-status", "account-presence", "account-about", "account-server-tag"]);
  for (const id of PROFILE_EDITOR_ORDER) assert.ok(html.includes(`id="${id}"`), id);
  assert.match(rule(`${R} .p-editor > .profile-fields`), /grid-template-columns: minmax\(0, 1fr\) minmax\(0, 1fr\);\n  gap: 14px;/);
  assert.ok(css.includes(`${R} .p-editor > .profile-fields > .field-label:has(#account-about),`));
  assert.match(rule(`${R} .p-editor > .profile-fields > .field-label:has(#account-server-tag)`), /grid-column: 1;/);
});

test("the banner runs edge to edge with the keys on its fade and Save on the bottom edge", () => {
  assert.match(rule(`${R} .p-editor > .profile-banner`), /height: 112px;\n  margin: 0 -24px;/);
  assert.ok(rule(`${R} .p-editor > .profile-banner`).includes("linear-gradient(rgb(0 0 0) 38%, transparent 92%)"));
  assert.match(rule(`${R} .p-editor > .role-banner-tools`), /top: 68px;\n  right: 24px;/);
  assert.match(rule(`${R} .p-editor > .role-banner-tools > .key.sq`), /width: 34px;\n  height: 34px;/);
  assert.match(rule(`${R} .p-editor > .role-banner-tools > #account-banner-remove`), /order: 1;/);
  assert.match(rule(`${R} .p-editor > .profile-body`), /gap: 14px;\n  margin-top: -22px;\n  padding: 0 2px 18px;/);
  assert.ok(css.includes(`${R} .p-editor > .profile-fields > .form-actions {\n  position: absolute;\n  right: 24px;\n  bottom: -20px;`));
  assert.match(rule(`${R} .set-pane.set-editor`), /padding: 0 24px 40px;/);
  assert.ok(glue.includes('["account-save", "key primary", ""]'));
  assert.ok(glue.includes("banner.after(tools)"));
});

test("borders and effects are glass tiles and the chosen one wears the ring", () => {
  assert.match(rule(`${R} .p-editor .look-grid`), /grid-template-columns: repeat\(9, minmax\(0, 1fr\)\);\n  gap: 6px;/);
  assert.match(rule(`${R} .p-editor #account-effect-grid`), /grid-template-columns: repeat\(5, minmax\(0, 1fr\)\);\n  gap: 10px;/);
  assert.match(rule(`${R} .p-editor .look-option`), /padding: 9px 0 8px;\n  border: 0;\n  border-radius: 16px;/);
  assert.match(rule(`${R} .p-editor .look-option[aria-checked="true"]`), /outline: var\(--ring-on\);\n  outline-offset: 2px;/);
  assert.match(rule(`${R} .p-editor .look-swatch`), /height: 44px;\n  border-radius: 11px;/);
  assert.ok(glue.includes('<svg class="set-scene theme-dark"'));
});

test("presence dots, the pronouns line and the About me count", () => {
  assert.equal(presenceDotClass("pres-dot", "online"), "pres-dot");
  assert.equal(presenceDotClass("pres-dot", "idle"), "pres-dot idle");
  assert.equal(presenceDotClass("set-avpres", "do_not_disturb"), "set-avpres dnd");
  assert.equal(presenceDotClass("set-avpres", "invisible"), "set-avpres off");
  assert.equal(presenceDotClass("pres-dot", "unknown"), "pres-dot");
  assert.equal(aboutCount("", 190), "0 / 190");
  assert.equal(aboutCount("Building the new look", 190), "21 / 190");
  assert.equal(aboutCount(null, 190), "0 / 190");
  assert.match(rule(`${R} .p-editor .field-emoji`), /padding: 14px 8px 50px 18px;\n  border-radius: 24px;/);
  assert.ok(css.includes(`${R} .p-editor .set-pronouns:not(:empty)::before {`));
});

test("the phone layout keeps its look", () => {
  assert.ok(css.includes(`${SLIM} .set-avw { display: contents; }`));
  for (const part of [".set-avpres", ".set-pronouns", ".set-count", ".set-scene", ".set-pick > .pres-dot", "#account-avatar-btn > .i", "#account-banner-upload > .i"]) {
    assert.ok(css.includes(`${SLIM} ${part}`), part);
  }
  assert.ok(glue.includes('document.documentElement.dataset.fit !== "slim"'));
  assert.ok(glue.includes("if (!roomy && tools.parentNode !== banner) banner.appendChild(tools);"));
  assert.ok(glue.includes("if (!roomy && avatarKey.parentNode !== body) body.appendChild(avatarKey);"));
});
