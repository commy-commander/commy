import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { EVENT_HEAD_KEYS, EVENT_HEAD_ICONS, eventActionKey, eventAnswerMine, eventDeleteIcon } = require("../scripts/studio-events.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const app = read("../scripts/app.js");
const glue = read("../scripts/studio-events.js");
const css = read("../styles/studio-events.css");
const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const selectors = [...css.replace(/@media[^{]+\{/g, "").matchAll(/(^|\})\s*([^{}@]+)\{/g)]
  .flatMap(match => match[2].split(/,\n/).map(part => part.trim()).filter(Boolean));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("the events sheet loads with the new look, roomy only, from the tokens", () => {
  assert.ok(selectors.length > 40);
  for (const selector of selectors) assert.ok(selector.startsWith(R), selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
  assert.doesNotMatch(glue, /\/\/|\/\*/);
  const anchor = '  <link rel="stylesheet" id="studio-desktop" href="styles/studio.css" media="not all" />\n';
  assert.ok(html.includes(`${anchor}  <link rel="stylesheet" id="studio-events" href="styles/studio-events.css" media="not all" />\n`));
  assert.ok(html.includes('  <script src="scripts/studio-room.js"></script>\n  <script src="scripts/studio-events.js"></script>\n'));
  assert.ok(html.indexOf('<script src="scripts/studio-events.js"></script>') < html.indexOf('<script src="scripts/app.js"></script>'));
  assert.equal(app.split("  if (studioLook()) studioEvents();\n").length, 2);
  assert.ok(app.indexOf("studioEvents();") > app.indexOf("  if (studioLook()) studioFloat();"));
  assert.ok(glue.includes('document.documentElement.dataset.fit !== "slim"'));
  assert.ok(glue.includes('attributeFilter: ["data-fit"]'));
});

test("the sheet is a 640 slab with tabs in a slot and glass keys in the head", () => {
  assert.match(rule(`${R} .events-panel`), /width: min\(640px, 100%\);/);
  assert.match(rule(`${R} .events-panel`), /padding: 18px 18px 14px;/);
  assert.match(rule(`${R} .events-panel`), /border-radius: var\(--radius-xl\);/);
  assert.match(rule(`${R} .events-head h2`), /font: 700 20px\/1\.2 var\(--font-display\);/);
  assert.match(rule(`${R} .events-tabs`), /box-shadow: var\(--slot\);/);
  assert.match(rule(`${R} .events-tab`), /height: 30px;/);
  assert.match(rule(`${R} .events-tab.active,\n${R} .events-tab.active:hover`), /outline: var\(--ring-on\);/);
  assert.match(rule(`${R} #events-close.key.sq,\n${R} .events-panel .cal-head > .key.sq`), /width: 36px;/);
  assert.deepEqual(EVENT_HEAD_KEYS, { "event-create": "key sm", "events-close": "key sq" });
  assert.deepEqual(EVENT_HEAD_ICONS, { "event-create": "plus" });
});

test("each event is a card with a date tile, the live tag and ringed answers", () => {
  assert.match(rule(`${R} .events-body > .event-card`), /border-radius: 18px;/);
  assert.match(rule(`${R} .events-body > .event-card`), /box-shadow: inset 0 0 0 1px var\(--line\);/);
  assert.match(rule(`${R} .event-card > .event-date`), /width: 52px;\n  height: 58px;/);
  assert.match(rule(`${R} .event-date-month`), /color: var\(--accent-lit\);/);
  assert.match(rule(`${R} .event-date-day`), /font: 700 22px\/1 var\(--font-display\);/);
  assert.match(rule(`${R} .event-when.live`), /background: var\(--live\);/);
  assert.match(rule(`${R} .event-when.live`), /color: var\(--live-ink\);/);
  assert.match(rule(`${R} .event-main > .event-title`), /font: 700 17px\/1\.25 var\(--font-display\);/);
  assert.match(rule(`${R} .event-counts`), /font: 500 12\.5px\/1\.2 var\(--font\);/);
  assert.match(rule(`${R} .event-actions > .key.sm.mine`), /box-shadow: inset 0 0 0 1\.5px var\(--accent-lit\);/);
  assert.match(rule(`${R} .event-actions > .icon-btn`), /border-radius: 50%;/);
  assert.equal(eventActionKey(0, false, true), "key sm");
  assert.equal(eventActionKey(1, false, true), "key sm");
  assert.equal(eventActionKey(2, false, true), "key sm primary");
  assert.equal(eventActionKey(3, true, true), "");
  assert.equal(eventActionKey(0, false, false), "");
  assert.equal(eventAnswerMine(1, "btn-small btn-primary", true), true);
  assert.equal(eventAnswerMine(1, "btn-small btn-secondary", true), false);
  assert.equal(eventAnswerMine(2, "btn-primary btn-small", true), false);
  assert.equal(eventAnswerMine(0, "btn-small btn-primary", false), false);
  assert.equal(eventDeleteIcon(true), "trash-2");
  assert.equal(eventDeleteIcon(false), "trash");
});

test("the calendar has 78 tiles, a filled today and a ringed picked day", () => {
  assert.match(rule(`${R} .cal-month`), /font: 700 17px\/1 var\(--font-display\);/);
  assert.match(rule(`${R} .cal-dow`), /font: 600 12px\/1 var\(--font\);/);
  assert.match(rule(`${R} .cal-day`), /height: 78px;/);
  assert.match(rule(`${R} .cal-day`), /border-radius: 12px;/);
  assert.match(rule(`${R} .cal-day.today .cal-num`), /background: var\(--accent\);/);
  assert.match(rule(`${R} .cal-day.selected,\n${R} .cal-day.selected:hover`), /box-shadow: inset 0 0 0 1\.5px var\(--accent-lit\);/);
  assert.match(rule(`${R} .cal-event`), /background: color-mix\(in oklab, var\(--accent\) 34%, transparent\);/);
  assert.match(rule(`${R} .cal-day.other`), /background: none;/);
});

test("no events shows the 64 well with the app's words", () => {
  assert.match(rule(`${R} .events-empty > svg`), /padding: 18px;/);
  assert.match(rule(`${R} .events-empty > svg`), /width: 28px !important;/);
  assert.match(rule(`${R} .events-empty > h3`), /font: 700 17px\/1\.2 var\(--font-display\);/);
  assert.match(rule(`${R} .events-empty > p`), /max-width: 340px;/);
  assert.match(app, /"Schedule one so people know when to show up\." : "Check back later\."/);
});

test("the events glue leaves the phone layout to the phone's own dressing", () => {
  assert.ok(glue.includes('if (!body.querySelector("[data-studio-event]")) return;'));
  assert.ok(glue.includes("renderEvents();"));
  assert.ok(glue.includes('if (!key.dataset.studioEvent) return;'));
  assert.ok(glue.includes('if (phone) key.classList.add("key");'));
  assert.ok(glue.includes('observe(create, { attributes: true, attributeFilter: ["class"] })'));
});
