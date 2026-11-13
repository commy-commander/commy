import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const touch = read("styles", "phone-touch.css");
const forum = read("styles", "phone-forum.css");
const viewer = read("styles", "phone-viewer.css");
const glue = read("scripts", "studio-phone.js");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';

const rule = (css, selector) => {
  const at = css.indexOf(`${SLIM} ${selector} {`);
  assert.notEqual(at, -1, selector);
  return css.slice(at, css.indexOf("}", at));
};

test("the presence sheet keeps its long lines inside a small phone", () => {
  assert.match(rule(touch, ".pt-pick-step"), /grid-template-columns: minmax\(0, 1fr\);/);
  assert.match(rule(touch, ".pt-pick .srow small"), /overflow: hidden; text-overflow: ellipsis;/);
});

test("the forum pencil is a full tap target and the waiting bar stays off the forum", () => {
  assert.match(rule(forum, "#forum-posts .forum-edit"), /top: 0;\n\s*right: 0;[\s\S]*width: 44px;\n\s*height: 44px;/);
  assert.match(forum, /\.main:has\(> #forum-view:not\(\.hidden\)\) #awaited-bar \{ display: none; \}/);
});

test("the image gate wraps its words above the Load key", () => {
  assert.match(rule(touch, ".msg .image-gate"), /flex-wrap: wrap;/);
  assert.match(rule(touch, ".msg .image-gate-text"), /flex: 1 0 100%;/);
  assert.match(touch, /\.msg \.image-gate-hint \{ white-space: normal; \}|\.msg \.image-gate-title,\n[^{]*\.msg \.image-gate-hint \{ white-space: normal; \}/);
  assert.match(rule(touch, ".msg .image-gate-load"), /height: 40px;.*border-radius: 999px;/);
});

test("small words that open things carry a 44 point tap area", () => {
  assert.match(rule(viewer, "#lightbox-source"), /position: relative;/);
  assert.match(rule(viewer, "#lightbox-source::after"), /content: "";\n\s*position: absolute;\n\s*inset: -15px -10px;/);
  assert.match(rule(touch, ".acct .t .presence::after"), /inset: -15px -10px;/);
  assert.match(rule(touch, ".pt-profile .in .e::after"), /inset: -13px -11px;/);
});

test("an empty display name shows the username as its placeholder", () => {
  assert.match(glue, /\$\("pt-prof-dn"\)\.value = saved\.name;\n\s*\$\("pt-prof-dn"\)\.placeholder = \(user && user\.username\) \|\| "";/);
  assert.match(rule(touch, ".pt-profile .in input::placeholder"), /color: var\(--text-faint\); opacity: 1;/);
});

test("an off phone switch at Day is a sunken track, not white on a white sheet", () => {
  const day = 'html[data-look="studio"][data-fit="slim"][data-theme="light"]';
  const sunk = "background: var(--bg-active); box-shadow: inset 0 1px 2px rgb(var(--dark) / .45), 0 1px 0 rgb(var(--lamp) / .5);";
  assert.ok(touch.includes(`${day} :not([aria-checked="true"]) > .tog { ${sunk} }`));
  assert.ok(read("styles", "phone-settings-page.css").includes(`${day} .settings-layer .settings-content input.switch:not(:checked) { ${sunk} }`));
});

test("a dialog opened while another phone layer is still stepping back waits for that step before taking its own", () => {
  const dialogs = read("scripts", "phone-dialogs.js");
  assert.match(dialogs, /if \(step === "push" && phoneDialogs\.moving\) \{\n\s*phoneDialogs\.waiting = true;\n\s*return;\n\s*\}/);
  assert.match(dialogs, /for \(const name of \["back", "go"\]\)/);
  assert.match(dialogs, /if \(waited\) \{\n\s*phoneDialogStep\(\);\n\s*return;\n\s*\}/);
});

test("the profile card's More menu is the same hold sheet as the server and channel menus", () => {
  const glue = read("scripts", "phone-menus.js");
  const css = read("styles", "phone-menus.css");
  assert.match(glue, /const PHONE_MENUS = \["server-menu", "channel-menu", "fp-menu", "room-member-menu"\];/);
  assert.match(glue, /"Report": "flag",/);
  assert.ok(css.includes(`${SLIM} #fp-menu > .arow:hover`));
  assert.ok(css.includes(`${SLIM} #fp-menu > .arow.danger:hover`));
});

test("the drive channel hides the waiting bar like the forum does", () => {
  assert.ok(read("styles", "phone-drive.css").includes(`${SLIM} .main:has(> #drive-view:not(.hidden)) #awaited-bar { display: none; }`));
});

test("a toast wears the new look on the phone and sits above a dialog's answers", () => {
  assert.match(rule(touch, ".toast"), /border-radius: 24px;[\s\S]*bottom: calc\(var\(--hz-line\) \+ 68px\);/);
  assert.ok(touch.includes(`${SLIM} body:has(> #modal-backdrop:not(.hidden)) > .toast { bottom: calc(var(--keyboard-height, 0px) + env(safe-area-inset-bottom) + 104px); }`));
});

test("small keys reach a 44 point tap area", () => {
  const dialogs = read("styles", "phone-dialogs.css");
  assert.ok(dialogs.includes(`${SLIM} .modal.dialog .key.latch { min-height: 44px; }`));
  assert.ok(dialogs.includes(`${SLIM} .modal.dialog.dc > .signin-back::after { content: ""; position: absolute; inset: -15px -10px; }`));
  assert.ok(dialogs.includes(`${SLIM} .modal.dialog .pin-unpin::after { content: ""; position: absolute; inset: -10px; }`));
  assert.ok(forum.includes(`${SLIM} #forum-view .forum-mode::after { content: ""; position: absolute; inset: -4px -2px; }`));
  assert.ok(touch.includes(`${SLIM} .stage-ink-tools > .stage-btn { width: 44px; min-width: 44px; }`));
});

test("Server Settings and Edit Channel from the menus open on the list, and the server overview card is titled", () => {
  const settings = read("scripts", "phone-settings.js");
  assert.match(settings, /const PHONE_SETTINGS_FROM_MENUS = \{ "sm-settings": "server-settings", "chm-settings": "channel-settings" \};/);
  assert.match(settings, /if \(studioSlim\(\)\) settingsPhoneView\(layer, "menu"\);/);
  assert.match(read("scripts", "phone-settings-page.js"), /pane\.prepend\(el\("h2", "settings-subhead set-head", "Server"\)\);/);
});

test("a long toast on the phone wraps to three lines and stays above the tab bar", () => {
  assert.match(rule(touch, ".toast"), /width: max-content;[\s\S]*padding: 12px 22px 12px 16px;[\s\S]*border-radius: 24px;/);
  assert.match(rule(touch, ".toast > .toast-text"), /-webkit-line-clamp: 3;[\s\S]*white-space: normal;/);
});

test("the reply chip takes the waiting bar's place, and Notify when online keeps the card open", () => {
  assert.ok(touch.includes(`${SLIM} .main:has(> .reply-bar:not(.hidden)) > #awaited-bar { display: none; }`));
  const app = read("scripts", "app.js");
  assert.match(app, /const item = \(label, icon, onClick, cls = "", keep\) => \{[\s\S]{0,200}closeFloating\(keep\);/);
  assert.match(app, /item\("Notify when online", "signal", \(\) => toggleFriendOnlinePick\(person\.id\), "ctx-check" \+ \(wants \? " checked" : ""\), keepOpen\);/);
});
