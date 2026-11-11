import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { dressPhoneForum } = require("../scripts/phone-forum.js");
const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-forum.css");
const glue = read("scripts", "phone-forum.js");
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';

const rule = selector => {
  const at = css.indexOf(`${SLIM} ${selector} {`);
  assert.notEqual(at, -1, selector);
  return css.slice(at, css.indexOf("}", at));
};

test("every forum rule is for the phone layout of the new look", () => {
  const selectors = css.match(/(?:^|\})\s*([^{}@]+)\{/g) || [];
  assert.ok(selectors.length > 40);
  for (const raw of selectors) {
    for (const selector of raw.replace(/^\}/, "").replace(/\{$/, "").replace(/\([^()]*\)/g, "").split(",")) {
      assert.ok(selector.trim().startsWith(SLIM), selector.trim());
    }
  }
});

test("the forum sheet uses tokens only, with no comments", () => {
  assert.doesNotMatch(css, /#[0-9a-fA-F]{3,8}\b(?![-\w])/);
  assert.doesNotMatch(css, /\/\*/);
  assert.doesNotMatch(glue, /\/\/|\/\*/);
});

test("the forum styles load right after the phone styles, off until the look turns them on", () => {
  assert.match(html, /id="studio-phone" href="styles\/phone-touch\.css" media="not all" \/>\n(\s*<link rel="stylesheet" id="studio-phone-[a-z-]+" href="styles\/phone-[a-z-]+\.css" media="not all" \/>\n)*\s*<link rel="stylesheet" id="studio-phone-forum" href="styles\/phone-forum\.css" media="not all" \/>/);
  assert.match(html, /<script src="scripts\/studio-phone\.js"><\/script>\n(\s*<script src="scripts\/phone-[a-z-]+\.js"><\/script>\n)*\s*<script src="scripts\/phone-forum\.js"><\/script>/);
});

test("the forum is one slab under the head, with New post as the primary capsule in the head", () => {
  assert.match(rule("#forum-view"), /border-radius: var\(--radius-xl\)/);
  assert.match(rule("#forum-view"), /margin: 6px 10px/);
  const add = rule("#forum-new");
  assert.match(add, /height: 44px/);
  assert.match(add, /padding: 0 16px 0 13px/);
  assert.match(add, /background: var\(--pillow-top\), var\(--accent\)/);
  assert.match(add, /top: -58px/);
});

test("the tag chips scroll and fade at the right edge, the layout slot stays", () => {
  const tags = rule("#forum-tags");
  assert.match(tags, /overflow-x: auto/);
  assert.match(tags, /mask-image: linear-gradient\(90deg, black 84%, transparent\)/);
  assert.match(rule("#forum-tags .forum-chip"), /height: 40px/);
  assert.match(rule("#forum-tags .forum-chip.picked"), /inset 0 0 0 1\.5px var\(--accent-lit\)/);
  assert.match(rule("#forum-view .forum-mode"), /width: 40px;\n  height: 36px/);
  assert.match(rule("#forum-view .forum-mode.active"), /outline: var\(--ring-on\)/);
});

test("posts are cards with the title on its own line, tags under it, then the meta line", () => {
  assert.match(rule("#forum-posts .forum-post"), /border-radius: 18px/);
  assert.match(rule("#forum-posts .forum-card"), /border-radius: 18px/);
  const title = rule("#forum-posts .forum-post-title");
  assert.match(title, /flex: 1 0 100%/);
  assert.match(title, /font: 600 16px\/20px var\(--font\)/);
  assert.match(rule("#forum-posts .forum-meta"), /font: 500 12\.5px\/1\.2 var\(--font\)/);
  assert.match(rule("#forum-posts.gallery"), /repeat\(2, minmax\(0, 1fr\)\)/);
  assert.match(rule("#forum-posts .forum-card-excerpt"), /-webkit-line-clamp: 4/);
});

test("no posts shows the quiet well with the forum icon", () => {
  const well = rule("#forum-posts .forum-empty .channel-intro-mark");
  assert.match(well, /width: 64px;\n  height: 64px/);
  assert.match(well, /box-shadow: var\(--slot\)/);
  assert.match(rule("#forum-posts .forum-empty h2"), /font: 700 17px\/1\.2 var\(--font-display\)/);
});

test("New post is a tall sheet with a grab, capsule slots and two 52 capsules", () => {
  const sheet = rule("#modal-forum-post");
  assert.match(sheet, /height: 640px/);
  assert.match(sheet, /var\(--sheet-bg\)/);
  assert.match(rule("#modal-forum-post::before"), /width: 38px;\n  height: 5px/);
  assert.match(rule("#forum-post-heading"), /font: 700 22px\/1\.2 var\(--font-display\)/);
  assert.match(rule("#modal-forum-post .field-label"), /font: 600 13px\/1 var\(--font\)/);
  assert.match(rule("#forum-post-title"), /height: 48px/);
  assert.match(css, /#forum-post-body \{\n  display: block;\n  height: 96px/);
  assert.match(rule("#forum-post-tags-left"), /color: var\(--text-faint\)/);
  assert.match(rule("#forum-post-tags .access-chip.picked"), /var\(--bg-active\)/);
  assert.match(css, /#forum-post-save \{\n  height: 52px/);
  assert.match(css, /#forum-post-cancel \{\n  background: var\(--k-plain\);/);
  assert.match(css, /#forum-post-save \{\n  background: var\(--pillow-top\), var\(--accent\);/);
});

test("the dressing only touches the forum keys in the phone layout", () => {
  assert.match(glue, /studioSlim\(\)/);
  assert.match(glue, /studioIconMarkup\("plus", "pf-plus"\)/);
  assert.match(glue, /studioIconMarkup\("layers", "pf-layers"\)/);
  assert.match(glue, /setIcon\(gallery, "grid", 18\)/);
  assert.equal(typeof dressPhoneForum, "function");
  assert.doesNotThrow(() => dressPhoneForum());
});
