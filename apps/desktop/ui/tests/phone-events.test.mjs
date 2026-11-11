import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { PHONE_EVENT_DOTS, PHONE_EVENT_FIELDS, phoneEventWhen, phoneEventDots, phoneEventSpareWeeks, phoneEventBackCloses, phoneEventStepsBack } = require("../scripts/phone-events.js");
const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-events.css");
const glue = read("scripts", "phone-events.js");
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';

const selectors = text => {
  const found = [];
  const body = text.replace(/@keyframes[^{]*\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, "");
  for (const match of body.matchAll(/([^{}]+)\{[^{}]*\}/g)) {
    for (const part of match[1].split(/,(?![^()]*\))/)) found.push(part.trim());
  }
  return found.filter(Boolean);
};

const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, `missing rule ${selector}`);
  return css.slice(start, css.indexOf("}", start));
};

test("every events rule is for the new look's phone layout only", () => {
  const all = selectors(css);
  assert.ok(all.length > 40);
  for (const selector of all) assert.ok(selector.startsWith(SLIM), selector);
});

test("the events sheet uses tokens, not raw colours, and carries no comments", () => {
  assert.doesNotMatch(css, /#[0-9a-fA-F]{3,8}\b(?![-\w])/);
  assert.doesNotMatch(css, /\/\*/);
  assert.doesNotMatch(glue, /\/\*|^\s*\/\//m);
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
});

test("the stylesheet sits right after the phone stylesheet, and the script after the phone script", () => {
  assert.match(html, /id="studio-phone" href="styles\/phone-touch\.css" media="not all" \/>\n(\s*<link rel="stylesheet" id="studio-phone-[a-z-]+" href="styles\/phone-[a-z-]+\.css" media="not all" \/>\n)*\s*<link rel="stylesheet" id="studio-phone-events" href="styles\/phone-events\.css" media="not all" \/>/);
  assert.match(html, /<script src="scripts\/studio-phone\.js"><\/script>\n(\s*<script src="scripts\/phone-[a-z-]+\.js"><\/script>\n)*\s*<script src="scripts\/phone-events\.js"><\/script>/);
});

test("the screen is pushed: a back key, Events with the server under it, and one slab", () => {
  assert.match(glue, /"Events"/);
  assert.match(glue, /aria-label", "Back"/);
  assert.match(rule(`${SLIM} #events-layer .pe-title b`), /font: 700 22px\/22px var\(--font-display\)/);
  assert.match(rule(`${SLIM} #events-layer .pe-title small`), /font: 400 13\.5px\/13\.5px var\(--font\)/);
  assert.match(rule(`${SLIM} #events-layer .pe-slab`), /margin: 6px 10px -40px;[\s\S]*border-radius: var\(--radius-xl\)/);
  assert.match(rule(`${SLIM} #events-layer :is(#events-title, #events-close)`), /display: none/);
});

test("Create Event is a glass capsule key in the head", () => {
  const key = rule(`${SLIM} #events-layer #event-create.key`);
  assert.match(key, /height: 44px/);
  assert.match(key, /padding: 0 16px 0 13px/);
  assert.match(key, /border-radius: 999px/);
  assert.doesNotMatch(key, /--accent/);
  assert.match(glue, /create\.classList\.add\("key", "pe-key"\)/);
  assert.match(glue, /phoneEventIcon\("plus"\)/);
});

test("Upcoming and Calendar sit in a slot, 40 tall, the open one ringed", () => {
  assert.match(rule(`${SLIM} #events-layer .events-tabs`), /background: var\(--input-bg\);\n {2}box-shadow: var\(--slot\)/);
  assert.match(rule(`${SLIM} #events-layer .events-tab`), /height: 40px/);
  assert.match(rule(`${SLIM} #events-layer .events-tab.active`), /outline: var\(--ring-on\)/);
});

test("an event card wraps its foot: the count line on its own row, then 40 tall keys", () => {
  assert.match(rule(`${SLIM} #events-body .event-card`), /grid-template-columns: 48px minmax\(0, 1fr\);[\s\S]*border-radius: 18px/);
  assert.match(rule(`${SLIM} #events-body .event-date`), /height: 54px/);
  assert.match(rule(`${SLIM} #events-body .event-foot`), /grid-column: 1 \/ -1;[\s\S]*flex-wrap: wrap/);
  assert.match(rule(`${SLIM} #events-body .event-counts`), /flex: 1 0 100%/);
  assert.match(rule(`${SLIM} #events-body .event-foot .key.sm`), /height: 40px/);
  assert.match(rule(`${SLIM} #events-body .event-foot .key.sm.mine`), /box-shadow: inset 0 0 0 1\.5px var\(--accent-lit\)/);
  assert.match(rule(`${SLIM} #events-body .event-foot .pe-ibtn`), /width: 40px;\n {2}height: 40px;\n {2}border-radius: 50%/);
  assert.match(rule(`${SLIM} #events-body .event-foot .pe-push`), /margin-left: auto/);
  assert.match(glue, /phoneEventIcon\("check"\)/);
  assert.match(glue, /"trash-2"/);
  assert.match(glue, /"volume-2"/);
});

test("happening now is the filled live tag", () => {
  const live = rule(`${SLIM} #events-body .event-when.live`);
  assert.match(live, /height: 22px/);
  assert.match(live, /background: var\(--live\)/);
  assert.match(live, /color: var\(--live-ink\)/);
});

test("the calendar has 46 tall day tiles, today in a filled dot, the picked day ringed", () => {
  assert.match(rule(`${SLIM} #events-body .cal-day`), /height: 46px/);
  assert.match(rule(`${SLIM} #events-body .cal-day.today .cal-num`), /width: 24px|flex: 0 0 24px/);
  assert.match(rule(`${SLIM} #events-body .cal-day.today .cal-num`), /background: var\(--accent\)/);
  assert.match(rule(`${SLIM} #events-body .cal-day.selected`), /box-shadow: inset 0 0 0 1\.5px var\(--accent-lit\)/);
  assert.match(rule(`${SLIM} #events-body .pe-dots i`), /width: 5px;\n {2}height: 5px;[\s\S]*background: var\(--accent-lit\)/);
  assert.match(glue, /charAt\(0\)/);
});

test("a day shows at most three dots", () => {
  assert.equal(PHONE_EVENT_DOTS, 3);
  assert.equal(phoneEventDots(0, false), 0);
  assert.equal(phoneEventDots(1, false), 1);
  assert.equal(phoneEventDots(2, false), 2);
  assert.equal(phoneEventDots(2, true), 3);
});

test("weeks wholly in the next month are left off the phone calendar", () => {
  const november = Array.from({ length: 42 }, (_, i) => i >= 30);
  assert.deepEqual(phoneEventSpareWeeks(november), [35, 36, 37, 38, 39, 40, 41]);
  const august = Array.from({ length: 42 }, (_, i) => i < 6 || i >= 37);
  assert.deepEqual(phoneEventSpareWeeks(august), []);
  const twoSpare = Array.from({ length: 42 }, (_, i) => i >= 28);
  assert.deepEqual(phoneEventSpareWeeks(twoSpare), [28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41]);
});

test("no events shows the 64 well", () => {
  assert.match(rule(`${SLIM} #events-body .pe-well`), /width: 64px;\n {2}height: 64px/);
  assert.match(glue, /phoneEventIcon\("calendar-days"\)/);
});

test("Create Event is a 640 tall sheet with a grab, capsule slots and two 52 keys", () => {
  const sheet = rule(`${SLIM} #modal-event`);
  assert.match(sheet, /height: 640px/);
  assert.match(sheet, /padding: 30px 18px 0/);
  assert.match(rule(`${SLIM} #modal-event::after`), /width: 38px;\n {2}height: 5px/);
  assert.match(rule(`${SLIM} #modal-event > h2`), /font: 700 22px\/1\.2 var\(--font-display\)/);
  assert.match(rule(`${SLIM} #modal-event .field-label`), /font: 600 13px\/1 var\(--font\);\n {2}color: var\(--text-muted\)/);
  assert.match(rule(`${SLIM} #modal-event #event-title`), /height: 48px/);
  assert.match(rule(`${SLIM} #modal-event .field-label > :is(.dfind, .dsel)`), /height: 48px/);
  assert.match(rule(`${SLIM} #modal-event #event-description`), /height: 96px/);
  assert.match(rule(`${SLIM} #modal-event > .modal-actions > button`), /height: 52px;[\s\S]*border-radius: 999px/);
  assert.match(rule(`${SLIM} #modal-event > .modal-actions > #event-save`), /var\(--accent\)/);
});

test("Starts and Ends show the date in words and open the phone's own picker", () => {
  assert.deepEqual(PHONE_EVENT_FIELDS, { "event-start": "calendar-days", "event-end": "clock" });
  assert.match(rule(`${SLIM} #modal-event .field-label > .dfind > input`), /opacity: 0/);
  assert.match(glue, /input\.showPicker\(\)/);
  assert.equal(phoneEventWhen(""), "");
  assert.equal(phoneEventWhen("not a date"), "");
  assert.equal(phoneEventWhen("2026-11-11T19:00", "en-US"), "Nov 11, 7:00 PM");
});

test("everything the phone adds is taken away again outside the phone layout", () => {
  assert.match(glue, /if \(!studioSlim\(\)\) \{\n {4}phoneEventsUndress\(\);/);
  assert.match(glue, /querySelectorAll\("#events-layer \.pe-x, #modal-event \.pe-x"\)/);
  assert.match(glue, /create\.classList\.add\("btn-primary", "btn-small"\)/);
});

test("the phone's Back closes the Create Event sheet first, then Events", () => {
  const both = { events: true, editor: true, eventsHeld: true, editorHeld: true };
  assert.deepEqual(phoneEventBackCloses({ pane: "list", ptEvents: true }, both), { editor: true, events: false });
  assert.deepEqual(phoneEventBackCloses({ pane: "list" }, { ...both, editor: false, editorHeld: false }), { editor: false, events: true });
  assert.deepEqual(phoneEventBackCloses({ pane: "list", ptEvents: true }, { ...both, editor: false, editorHeld: false }), { editor: false, events: false });
  assert.deepEqual(phoneEventBackCloses(null, { events: true, editor: false, eventsHeld: false, editorHeld: false }), { editor: false, events: false });
  assert.deepEqual(phoneEventBackCloses(null, { events: false, editor: false, eventsHeld: true, editorHeld: true }), { editor: false, events: false });
});

test("closing Events with its own keys steps its history entries back, both at once when both close", () => {
  assert.equal(phoneEventStepsBack("back", "back"), 2);
  assert.equal(phoneEventStepsBack("back", "keep"), 1);
  assert.equal(phoneEventStepsBack("none", "back"), 1);
  assert.equal(phoneEventStepsBack("push", "keep"), 0);
});

test("opening Events in the phone layout pushes one entry, and the sheet one more", () => {
  assert.match(glue, /layerStep\("ptEvents", events, phoneEventSteps\.events, history\.state\)/);
  assert.match(glue, /ptEvents: true \}, ""\)/);
  assert.match(glue, /layerStep\("ptEventEditor", editor, phoneEventSteps\.editor, history\.state\)/);
  assert.match(glue, /if \(back\) history\.go\(-back\)/);
  assert.match(glue, /window\.addEventListener\("popstate", phoneEventsOnBack\)/);
  assert.match(glue, /return studioSlim\(\) && !!layer/);
});
