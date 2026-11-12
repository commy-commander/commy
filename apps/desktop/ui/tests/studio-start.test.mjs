import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { START_KEYS, startStarState, startKeyFor, startGhostsWanted } = require("../scripts/studio-start.js");

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio-start.css");
const glue = read("../scripts/studio-start.js");
const app = read("../scripts/app.js");
const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const SLIM = 'html[data-look="studio"][data-fit="slim"]';
const selectors = [...css.replace(/@keyframes[^{]+\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, "").replace(/@media[^{]+\{/g, "").matchAll(/(^|\})\s*([^{}@]+)\{/g)]
  .flatMap(match => match[2].split(/,\n/).map(part => part.trim()).filter(Boolean));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};
const once = (text, part) => text.indexOf(part) >= 0 && text.indexOf(part) === text.lastIndexOf(part);

test("the first run sheet loads with the new look, roomy only, from the tokens", () => {
  const link = '<link rel="stylesheet" id="studio-start" href="styles/studio-start.css" media="not all" />';
  assert.ok(once(html, link));
  assert.ok(html.indexOf(link) > html.indexOf('id="studio-tokens"'));
  assert.ok(html.indexOf(link) < html.indexOf('id="studio-phone"'));
  assert.ok(selectors.length > 40);
  for (const selector of selectors.filter(s => !s.startsWith(SLIM))) assert.ok(selector.startsWith(R), selector);
  assert.deepEqual(selectors.filter(s => s.startsWith(SLIM)), [`${SLIM} :is(.start-room, .start-star, .start-find, .start-ghosts)`]);
  assert.match(rule(`${SLIM} :is(.start-room, .start-star, .start-find, .start-ghosts)`), /display: none;/);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
});

test("the glue loads once after the star and the room, and starts once after the floating parts", () => {
  const script = '<script src="scripts/studio-start.js"></script>';
  assert.ok(once(html, script));
  assert.ok(html.indexOf(script) > html.indexOf('<script src="scripts/star.js"></script>'));
  assert.ok(html.indexOf(script) > html.indexOf('<script src="scripts/studio-room.js"></script>'));
  const line = "  if (studioLook()) studioStart();\n";
  assert.ok(once(app, line));
  assert.ok(app.indexOf(line) > app.indexOf("  if (studioLook()) studioFloat();\n"));
  assert.doesNotMatch(glue, /\/\/|\/\*/);
});

test("the star is busy while Commy starts and waits for you everywhere else", () => {
  assert.equal(startStarState("startup"), "working");
  assert.equal(startStarState("reopen"), "waiting");
  assert.ok(glue.includes('startStar(intro, "waiting")'));
  assert.ok(glue.includes('startStar(home, "waiting", home.querySelector(":scope > .welcome-mark"))'));
});

test("the welcome keys become the bright key and a glass key, and Open Commy now a glass key", () => {
  assert.deepEqual(START_KEYS, { "btn-primary": "key primary", "btn-secondary": "key" });
  assert.equal(startKeyFor(["btn-primary"]), "key primary");
  assert.equal(startKeyFor(["btn-secondary", "x"]), "key");
  assert.equal(startKeyFor(["btn-link"]), null);
  assert.ok(glue.includes('studioKeyDress(skip, "key", roomy)'));
});

test("ghost rows show only for an empty list with nothing typed", () => {
  assert.equal(startGhostsWanted({ query: "", rows: 0, empty: true }), true);
  assert.equal(startGhostsWanted({ query: "  ", rows: 0, empty: true }), true);
  assert.equal(startGhostsWanted({ query: "no", rows: 0, empty: true }), false);
  assert.equal(startGhostsWanted({ query: "", rows: 2, empty: true }), false);
  assert.equal(startGhostsWanted({ query: "", rows: 0, empty: false }), false);
  assert.ok(glue.includes("for (let i = 0; i < 7; i++)"));
});

test("starting up is the guide's offline pane: a thin slab, the star at 58, the title at 22, keys across the edge", () => {
  const card = rule(`${R} .startup-card`);
  assert.match(card, /grid-template-columns: 58px minmax\(0, 1fr\);/);
  assert.match(card, /column-gap: 18px;/);
  assert.match(card, /width: min\(410px, 100%\);/);
  assert.match(card, /padding: 22px 24px 20px;/);
  assert.match(card, /var\(--panel-thin\);/);
  assert.match(card, /box-shadow: inset 0 0 0 1px var\(--rim-soft\), var\(--shadow-2\);/);
  assert.match(rule(`${R} .startup-card > .start-star`), /--star-size: 58px;/);
  assert.match(rule(`${R} .startup-card > .startup-logo`), /display: none;/);
  assert.match(rule(`${R} .startup-title`), /font: 700 22px\/1\.14 var\(--font-display\);/);
  assert.match(rule(`${R} .reopen-text`), /font: 400 15px\/1\.45 var\(--font\);/);
  assert.match(rule(`${R} .startup-step`), /height: 58px;/);
  assert.match(rule(`${R} .startup-step[data-state="working"] .startup-mark`), /var\(--yellow\)/);
  assert.match(rule(`${R} .startup-step[data-state="done"] .startup-mark`), /color: var\(--green\);/);
  assert.match(rule(`${R} .startup-name`), /font: 600 14\.5px\/1 var\(--font\);/);
  assert.match(rule(`${R} .startup-detail`), /font: 400 12\.5px\/1\.2 var\(--font\);/);
  assert.match(rule(`${R} .startup-skip.key`), /grid-column: 1 \/ -1;\n  grid-row: 4;\n  justify-self: end;\n  margin: 14px 0 0;/);
  assert.ok(css.includes(`${R} .startup-skip.key:not(.shown) { display: none; }`));
  assert.doesNotMatch(rule(`${R} .startup-skip.key`), /position: absolute|bottom: -/);
  assert.ok(css.includes("@keyframes start-wait { 50% { opacity: .35; } }"));
  assert.ok(css.includes(`@media (prefers-reduced-motion: reduce) {\n  ${R} .startup-step[data-state="working"] .startup-mark { animation: none; }\n}`));
});

test("the offline chip is a thin slab capsule", () => {
  const chip = rule(`${R} .offline-chip`);
  assert.match(chip, /height: 32px;/);
  assert.match(chip, /border: 0;/);
  assert.match(chip, /var\(--panel-thin\);/);
  assert.match(chip, /font: 500 12\.5px\/1 var\(--font\);/);
});

test("an empty channel is the guide's first day: the waiting star at 64 beside the words", () => {
  const C = `${R} .messages > .channel-intro:only-child`;
  assert.match(rule(C), /grid-template-columns: 64px minmax\(0, 1fr\);/);
  assert.match(rule(C), /column-gap: 20px;/);
  assert.match(rule(C), /padding: 0 0 20px 2px;/);
  assert.match(rule(`${C} > .start-star`), /--star-size: 64px;/);
  assert.match(rule(`${C} > .channel-intro-mark`), /display: none;/);
  assert.match(rule(`${C} > h2`), /font: 700 25px\/1\.14 var\(--font-display\);/);
  assert.match(rule(`${C} > p`), /max-width: 44ch;/);
  assert.match(rule(`${C} > p`), /margin: 9px 0 0;/);
  assert.match(rule(`${C} > p`), /font: 400 15px\/1\.45 var\(--font\);/);
  assert.match(rule(`${R} .channel-intro > .start-star`), /display: none;/);
});

test("the home with no servers and no messages uses the same words, the star and the keys", () => {
  assert.match(rule(`${R} .welcome-home > .welcome-mark`), /display: none;/);
  assert.match(rule(`${R} .welcome-home > .start-star`), /--star-size: 64px;/);
  assert.match(rule(`${R} .dm-empty > .dm-empty-art`), /width: 64px;\n  height: 64px;/);
  assert.match(rule(`${R} .messages > :is(.welcome-home, .dm-empty) > h3`), /font: 700 25px\/1\.14 var\(--font-display\);/);
  assert.match(rule(`${R} .welcome-actions`), /gap: 8px;/);
});

test("the conversation list on day one: a 36 capsule search and ghost rows that fade", () => {
  const find = rule(`${R} .sidebar-header > .dm-search`);
  assert.match(find, /height: 36px;/);
  assert.match(find, /border-radius: var\(--radius-pill\);/);
  assert.match(find, /font: 400 14px\/1 var\(--font\);/);
  assert.match(rule(`${R} .sidebar-header:has(> .dm-search:not(.hidden))`), /padding: 12px 21px 0 20px;/);
  assert.match(rule(`${R} .sidebar-header > .start-find`), /width: 18px;/);
  assert.match(rule(`${R} .sidebar-header > .dm-search.hidden ~ .start-find`), /display: none;/);
  assert.match(rule(`${R} .start-ghost`), /height: 44px;/);
  assert.match(rule(`${R} .start-ghost > i`), /width: 30px;\n  height: 30px;/);
  assert.match(rule(`${R} .start-ghosts`), /mask-image: linear-gradient\(180deg, black 10%, transparent 96%\);/);
  assert.match(rule(`${R} .start-ghosts + .dm-list-empty`), /font: 400 15px\/1\.45 var\(--font\);/);
});

test("offline is waiting, not yours: tungsten on the offline connection mark and the chip's dot, red words only for a real failure", () => {
  const sheet = css;
  const R = 'html[data-look="studio"]:not([data-fit="slim"])';
  assert.ok(sheet.includes(`${R} #startup.offline #startup-connection[data-state="failed"] .startup-mark {\n  background: var(--accent-2);\n  color: var(--accent-2-ink);`));
  assert.ok(sheet.includes(`${R} .offline-chip > .offline-dot { background: var(--accent-2); }`));
  assert.ok(sheet.includes(`${R} .startup-step[data-state="failed"] .startup-mark {\n  border: 0;\n  background: none;\n  box-shadow: none;\n  color: var(--danger-text);`));
  const code = read("../scripts/app.js");
  assert.ok(code.includes('$("startup").classList.toggle("offline", view.offline);'));
});
