import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("../styles/studio.css");
const app = read("../scripts/app.js");
const link = read("../scripts/invite-link.js");

const DESK = 'html[data-look="studio"]:not([data-fit="slim"])';
const block = width => {
  const start = css.indexOf(`@container icard (max-width: ${width}px) {`);
  assert.ok(start > 0);
  return css.slice(start, css.indexOf("\n}\n", start));
};

test("the server invite and the voice call invite are one card that answers to its own width", () => {
  const start = css.indexOf(`${DESK} .invite-card,\n${DESK} .voice-invite {`);
  assert.ok(start > 0);
  const card = css.slice(start, css.indexOf("}", start));
  for (const line of ["container: icard / inline-size;", "grid-template-columns: 44px minmax(0, 1fr) auto;", "width: 440px;", "max-width: 100%;", "border-radius: 24px;", "box-shadow: var(--pillow-card);"]) assert.ok(card.includes(line), line);
  assert.match(card, /background: radial-gradient\([^;]*var\(--accent\)[^;]*var\(--pillow-top\), var\(--bg-raised\);/);
  assert.ok(!css.includes("@container icard (min-width"));
});

test("under 340 the Join key drops under the words and fills the card, under 260 the mark goes", () => {
  const narrow = block(339);
  assert.ok(narrow.includes(`${DESK} .voice-invite-join { grid-column: 1 / -1; width: 100%; }`));
  assert.ok(narrow.includes(`${DESK} .voice-invite-body { grid-column: 2 / -1; }`));
  assert.match(narrow, /\.invite-card-sub \{[^}]*white-space: normal;[^}]*-webkit-line-clamp: 2; \}/);
  const tiny = block(259);
  assert.ok(tiny.includes(`${DESK} .voice-invite-mark { display: none; }`));
  assert.ok(tiny.includes(`${DESK} .voice-invite-status { flex: 1 0 100%; }`));
  assert.ok(css.indexOf("@container icard (max-width: 339px)") < css.indexOf("@container icard (max-width: 259px)"));
});

test("the title holds two lines at most and Join is the ultramarine key, 40 high and 84 wide at least", () => {
  assert.match(css, /\.voice-invite-title \{\n  overflow: visible;\n  color: var\(--header-primary\);\n  font: 600 14\.5px\/1\.25 var\(--font\);\n  text-wrap: balance;/);
  assert.ok(css.includes(`${DESK} .voice-invite-title { display: -webkit-box; overflow: hidden; -webkit-box-orient: vertical; -webkit-line-clamp: 2; }`));
  const start = css.indexOf(`${DESK} .voice-invite-join:hover {\n  display: inline-flex;`);
  assert.ok(start > 0);
  const key = css.slice(start, css.indexOf("}", start));
  for (const line of ["min-width: 84px;", "height: 40px;", "background: var(--pillow-top), var(--accent);", "color: var(--accent-ink);"]) assert.ok(key.includes(line), line);
  assert.ok(css.includes(`${DESK} .voice-invite-faces .avatar { width: 22px; height: 22px;`));
  assert.ok(css.includes(`${DESK} .voice-invite-kicker { color: var(--accent-lit); font: 600 12px/1 var(--font);`));
});

test("a call that is gone or full loses the wash, greys its mark and its Join key", () => {
  assert.ok(css.includes(`${DESK} .voice-invite:has(> .voice-invite-join:disabled) { background: var(--pillow-top), var(--bg-raised); }`));
  assert.ok(css.includes(`${DESK} .voice-invite:has(> .voice-invite-join:disabled) .voice-invite-mark { background: var(--bg-active); color: var(--text-muted); }`));
  assert.match(css, /\.voice-invite-join:disabled:hover \{\n  background: var\(--k-off\);\n  box-shadow: var\(--k-off-shadow\);/);
});

test("each card starts with its round mark in the new look, and the phone keeps its own call card", () => {
  assert.match(link, /function inviteCardMark\(\) \{\n  const mark = el\("div", "invite-card-mark"\);/);
  assert.match(link, /studio \? studioIconNode\("user-plus", 21, false\) : svgIcon\("serverAdd", 18\)/);
  assert.match(link, /const card = el\("div", "invite-card"\);\n  card\.appendChild\(inviteCardMark\(\)\);/);
  assert.match(app, /const card = el\("div", "invite-card"\);\n  if \(studioLook\(\)\) card\.appendChild\(inviteCardMark\(\)\);/);
  assert.match(app, /if \(studioLook\(\)\) \{\n    const mark = el\("div", "voice-invite-mark"\);\n    mark\.hidden = true;\n    mark\.appendChild\(studioIconNode\("volume-2", 21, false\)\);\n    card\.appendChild\(mark\);\n  \}\n  const body = el\("div", "voice-invite-body"\);/);
  assert.ok(!css.includes('[data-fit="slim"] .voice-invite-mark'));
  assert.match(css, /\.voice-invite-mark \{\n  display: grid;/);
});
