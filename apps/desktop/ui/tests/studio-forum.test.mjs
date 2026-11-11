import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { FORUM_KEYS, FORUM_ICONS, forumKeyNames } = require("../scripts/studio-forum.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const app = read("../scripts/app.js");
const glue = read("../scripts/studio-forum.js");
const css = read("../styles/studio-forum.css");
const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const SLIM = 'html[data-look="studio"][data-fit="slim"]';
const selectors = [...css.replace(/@media[^{]+\{/g, "").matchAll(/(^|\})\s*([^{}@]+)\{/g)]
  .flatMap(match => match[2].split(/,\n/).map(part => part.trim()).filter(Boolean));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("the forum view loads with the new look, roomy only, from the tokens", () => {
  const sheet = html.indexOf('<link rel="stylesheet" id="studio-forum" href="styles/studio-forum.css" media="not all" />');
  assert.ok(sheet > 0);
  assert.ok(html.includes('id="studio-profile-editor" href="styles/studio-profile-editor.css" media="not all" />\n  <link rel="stylesheet" id="studio-forum"'));
  assert.ok(selectors.length > 30);
  for (const selector of selectors) assert.ok(selector.startsWith(R) || (selector.startsWith(SLIM) && /display: none/.test(rule(selector))), selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
  assert.equal(/\/\/|\/\*/.test(glue), false);
});

test("the glue loads after the faces and starts once with the new look", () => {
  assert.ok(html.includes('<script src="scripts/studio-faces.js"></script>\n  <script src="scripts/studio-forum.js"></script>'));
  assert.ok(html.indexOf('<script src="scripts/studio-forum.js"></script>') < html.indexOf('<script src="scripts/app.js"></script>'));
  assert.equal(app.split("if (studioLook()) studioForum();").length, 2);
  assert.ok(app.indexOf("studioForum();") > app.indexOf("  if (studioLook()) studioFloat();"));
});

test("New post is the bright small key with plus, and the layout keys use list and layers", () => {
  assert.deepEqual(FORUM_KEYS, { "forum-new": "key sm primary" });
  assert.deepEqual(forumKeyNames(FORUM_KEYS["forum-new"], true), { key: true, sm: true, primary: true });
  assert.deepEqual(forumKeyNames(FORUM_KEYS["forum-new"], false), { key: false, sm: false, primary: false });
  assert.deepEqual(FORUM_ICONS.map(([id, icon]) => [id, icon]), [["forum-new", "plus"], ["forum-mode-gallery", "layers"]]);
  assert.match(rule(`${R} #forum-new.key.sm`), /padding: 0 14px 0 11px;/);
  assert.match(rule(`${R} #forum-mode-gallery > svg:not(.fv-i)`), /display: none;/);
  assert.match(rule(`${SLIM} .fv-i`), /display: none;/);
});

test("the bar holds tag chips, the layout slot and New post as the guide measures them", () => {
  assert.match(rule(`${R} .forum-toolbar`), /padding: 12px 16px 12px 28px;/);
  assert.match(rule(`${R} .forum-toolbar`), /box-shadow: none;/);
  assert.match(rule(`${R} .forum-chip`), /height: 32px;/);
  assert.match(rule(`${R} .forum-chip`), /padding: 0 14px;/);
  assert.match(rule(`${R} .forum-chip`), /font: 500 13\.5px\/1 var\(--font\);/);
  assert.match(rule(`${R} .forum-chip`), /box-shadow: inset 0 0 0 1px var\(--line\);/);
  assert.match(rule(`${R} .forum-chip.picked:hover`), /box-shadow: inset 0 0 0 1\.5px var\(--accent-lit\);/);
  assert.match(rule(`${R} .forum-modes`), /padding: 3px;/);
  assert.match(rule(`${R} .forum-modes`), /box-shadow: var\(--slot\);/);
  assert.match(rule(`${R} .forum-mode`), /width: 32px;\n  height: 28px;/);
  assert.match(rule(`${R} .forum-mode.active`), /outline: var\(--ring-on\);/);
});

test("list rows, gallery cards and the empty well follow the guide", () => {
  assert.match(rule(`${R} .forum-post`), /padding: 12px;/);
  assert.match(rule(`${R} .forum-post`), /border-radius: 14px;/);
  assert.match(rule(`${R} .forum-card.open:hover`), /box-shadow: inset 0 0 0 1\.5px var\(--accent-lit\);/);
  assert.match(rule(`${R} .forum-post-title`), /font: 600 15px\/1\.25 var\(--font\);/);
  assert.match(rule(`${R} .forum-pill`), /height: 22px;/);
  assert.match(rule(`${R} .forum-pill`), /font: 600 12px\/1 var\(--font\);/);
  assert.match(rule(`${R} .forum-meta`), /font: 500 12\.5px\/1\.2 var\(--font\);/);
  assert.match(rule(`${R} .forum-meta-author`), /font-weight: 600;/);
  assert.match(rule(`${R} .forum-edit`), /border-radius: 50%;/);
  assert.match(rule(`${R} .forum-posts.gallery`), /grid-template-columns: repeat\(2, minmax\(0, 1fr\)\);/);
  assert.match(rule(`${R} .forum-card`), /border-radius: 18px;/);
  assert.match(rule(`${R} .forum-card`), /padding: 14px;/);
  assert.match(rule(`${R} .forum-card-excerpt`), /font: 400 13\.5px\/1\.45 var\(--font\);/);
  assert.match(rule(`${R} .forum-empty .channel-intro-mark`), /width: 64px;\n  height: 64px;/);
  assert.match(rule(`${R} .forum-empty .channel-intro-mark`), /box-shadow: var\(--slot\);/);
  assert.match(rule(`${R} .forum-empty h2`), /font: 700 17px\/1\.2 var\(--font-display\);/);
  assert.match(rule(`${R} .forum-empty p`), /max-width: 340px;/);
});
