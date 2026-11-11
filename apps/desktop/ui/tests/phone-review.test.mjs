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
  assert.match(rule(touch, ".pt-profile .in .e::after"), /inset: -13px -10px;/);
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
