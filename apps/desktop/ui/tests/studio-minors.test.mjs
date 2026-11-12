import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { minorsPointAnchor, minorsBarDrops, minorsChatSizeLabel } = require("../scripts/studio-minors.js");
const { folderLabel } = require("../scripts/data-folder.js");

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio-minors.css");
const glue = read("../scripts/studio-minors.js");
const app = read("../scripts/app.js");
const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const selectors = [...css.matchAll(/(^|\})\s*([^{}@]+)\{/g)]
  .flatMap(match => match[2].split(/,\n/).map(part => part.trim()).filter(Boolean));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("the small fixes load with the new look, roomy only, from the tokens", () => {
  const sheet = '<link rel="stylesheet" id="studio-dialogs" href="styles/studio-dialogs.css" media="not all" />\n  <link rel="stylesheet" id="studio-minors" href="styles/studio-minors.css" media="not all" />';
  assert.ok(html.includes(sheet));
  assert.ok(html.includes('<script src="scripts/studio-dialogs.js"></script>\n  <script src="scripts/studio-minors.js"></script>'));
  assert.ok(html.indexOf("scripts/studio-minors.js") < html.indexOf("scripts/app.js"));
  assert.ok(selectors.length > 10);
  for (const selector of selectors) assert.ok(selector.startsWith(R), selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
  assert.doesNotMatch(glue, /\/\/|\/\*/);
});

test("app.js starts the glue once, after the floating parts", () => {
  const line = "  if (studioLook()) studioMinors();\n";
  assert.equal(app.split(line).length, 2);
  assert.ok(app.indexOf(line) > app.indexOf("  if (studioLook()) studioFloat();\n"));
});

test("nothing in the list is under 12 px, and no spaced capitals", () => {
  for (const selector of [".sidebar-link-badge", ".conn-badge", ".server-tag-emoji", ".role-banner-preview"]) {
    assert.match(rule(`${R} ${selector}`), /font-size: 12px;/, selector);
  }
  assert.match(rule(`${R} .msg-held`), /font-size: 12px;/);
  assert.ok(css.includes(`${R} .msg-pinned,\n${R} .msg-held {`));
  for (const selector of [".modal.dialog.make .kind-text .beta-tag", ".manage-row .kind", ".bot-card-key"]) {
    assert.match(rule(`${R} ${selector}`), /font-size: 12px;\n  letter-spacing: 0;\n  text-transform: none;/, selector);
  }
  assert.match(rule(`${R} .manage-row .kind::first-letter`), /text-transform: uppercase;/);
  assert.match(rule(`${R} .modal.dialog.make#modal-add-server > .modal-divider`), /font-size: 12px;\n  letter-spacing: 0;\n  text-transform: lowercase;/);
});

test("a right click on a message opens the More menu at the pointer", () => {
  const anchor = minorsPointAnchor(300, 200);
  const box = anchor.getBoundingClientRect();
  assert.equal(box.left, 300);
  assert.equal(box.bottom + 4, 200);
  assert.equal(anchor.closest(".x"), null);
  assert.ok(glue.includes('document.addEventListener("contextmenu", minorsMessageMenu, true);'));
  assert.ok(glue.includes('openMessageMenu(message, minorsPointAnchor(e.clientX, e.clientY));'));
  assert.ok(app.includes('add("more", "More", e => openMessageMenu(message, e.currentTarget));'));
});

test("the hover bar drops inside a message with no room above it", () => {
  assert.equal(minorsBarDrops(170, -16, 160), true);
  assert.equal(minorsBarDrops(176, -16, 160), false);
  assert.match(rule(`${R} .msg[data-minors-low] > .msg-actions`), /top: 4px;/);
  assert.match(read("../styles/styles.css"), /\.msg-actions \{\n  position: absolute;\n  top: -16px;/);
});

test("Storage shows the folder's own name, not the full path", () => {
  assert.equal(folderLabel("one\\two\\chat.commy.desktop"), "chat.commy.desktop");
  assert.equal(folderLabel("games\\Commy\\"), "Commy");
  assert.equal(folderLabel("/home/you/.local/share/chat.commy.desktop"), "chat.commy.desktop");
  assert.equal(folderLabel(""), "");
  assert.ok(html.includes('id="storage-open">Open Folder</button>'));
  assert.match(rule(`${R} .storage-path[data-minors-label]`), /font: 500 14px\/20px var\(--font\);/);
});

test("the Appearance size label is the size the new look draws", () => {
  assert.ok(read("../styles/studio.css").includes(`${R} .msg-content { font-size: max(12px, calc(var(--chat-font-size) * .9375));`));
  assert.equal(minorsChatSizeLabel(16), "15px");
  assert.equal(minorsChatSizeLabel(12), "12px");
  assert.equal(minorsChatSizeLabel(20), "19px");
  assert.ok(glue.includes("formatTime(noon.toISOString())"));
});

test("banner copy no longer promises a band in the channel list", () => {
  assert.ok(html.includes('<div class="setting-sub">A colour for the channel in the channel list.</div>'));
  assert.ok(html.includes('<div class="setting-sub">Optional picture shown at the top of the chat.</div>'));
  assert.doesNotMatch(html, /colour band behind the name/);
});

test("the Discover chips fade at the right edge and keep their hairline", () => {
  const discover = read("../styles/studio-discover.css");
  assert.ok(discover.includes("  mask-image: linear-gradient(90deg, black 88%, transparent), linear-gradient(black, black);\n  mask-size: 100% 100%, 100% 1px;"));
  assert.ok(discover.includes(`${R} .discover-chips::after {\n  content: "";\n  flex: none;\n  width: 12%;\n}`));
  assert.match(read("../styles/styles.css"), /\.discover-chips \{[^}]*overflow-x: auto;/);
});

test("Choose Roles rows are the guide's pills with a tick, not a radio circle", () => {
  const D = `${R} #modal-choose-roles.dialog`;
  assert.match(rule(`${D} .role-pick`), /border-radius: var\(--radius-pill\)/);
  assert.match(rule(`${D} .role-pick.held`), /box-shadow: inset 0 0 0 1\.5px var\(--accent-lit\)/);
  const check = rule(`${D} .role-pick > .role-pick-check`);
  assert.match(check, /border: 0/);
  assert.match(check, /background: none/);
  assert.match(check, /opacity: 0/);
  assert.match(rule(`${D} .role-pick.held > .role-pick-check`), /opacity: 1/);
});
