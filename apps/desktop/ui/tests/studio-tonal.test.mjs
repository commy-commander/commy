import { englishSource } from "./i18n-global.mjs";
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = path => englishSource(readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n"));
const css = read("../styles/studio-tonal.css");
const html = read("../index.html");
const app = read("../scripts/app.js");
const T = 'html[data-look="studio"]:root';
const line = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("the tonal sheet loads last before the contrast sheet and brings Geist with it", () => {
  const tonal = html.indexOf('id="studio-tonal" href="styles/studio-tonal.css"');
  assert.ok(tonal > 0);
  assert.ok(tonal < html.indexOf('id="studio-contrast"'));
  assert.match(css, /@font-face \{ font-family: "Geist"; src: url\(\.\.\/fonts\/geist-latin-wght-normal\.woff2\) format\("woff2"\);/);
  assert.match(css, /--font: "Geist"/);
});

test("keys that are on wear the periwinkle fill, and Cancel beside a red key goes plain", () => {
  assert.match(css, /--on-top: #7686ff;/);
  assert.match(css, /--on-bot: #5466f0;/);
  assert.match(css, /--on-flat: #6273f8;/);
  assert.match(css, /--accent: #6273f8;/);
  assert.match(css, /--accent: #5466f0;/);
  assert.match(css, /:has\(> \.key\.danger\) > \.key\.key\.primary:not\(\.danger\)/);
});

test("glass, grain and the horizon are gone from every pane", () => {
  assert.match(css, /--grain: none;/);
  assert.match(css, /\* \{ -webkit-backdrop-filter: none !important; backdrop-filter: none !important; \}/);
  assert.match(css, /\.hzn-line/);
});

test("an event card leads with its date tile, and a live one fills it and says Happening now", () => {
  assert.match(line(`${T} #events-body .event-card`), /border-radius: 22px; background: var\(--ev-card\);/);
  assert.match(line(`${T} #events-body .event-card.is-now`), /background: var\(--ev-now\);/);
  assert.match(app, /const card = el\("article", "event-card" \+ \(live \? " is-now" : ""\)\);/);
  assert.match(app, /el\("h3", "events-sec", t\("ui\.happening-now"\)\)|el\("h3", "events-sec", "Happening now"\)/);
  assert.doesNotMatch(app, /event-when live/);
});

test("Interested and Going are one switch, and the chosen side wears the check", () => {
  assert.match(line(`${T} #events-body .event-rsvp`), /border-radius: 999px; background: var\(--rsvp-well\);/);
  assert.match(app, /answers\.setAttribute\("role", "group"\);/);
  assert.match(app, /answer\.setAttribute\("aria-pressed", String\(mine\)\);/);
  assert.match(app, /if \(mine\) answer\.appendChild\(svgIcon\("check", 15\)\);/);
  assert.match(app, /more\.onclick = \(\) => openEventMenu\(event, more\);/);
});

test("the event description has four keys: Aa, emoji, GIF and stickers, and Aa opens bold, italic and strikethrough", () => {
  const bar = html.slice(html.indexOf('<div class="rich-bar"'), html.indexOf("</div>", html.indexOf('<div class="rich-bar"')));
  assert.equal((bar.match(/<button /g) || []).length, 4);
  for (const id of ["event-aa", "event-emoji", "event-gif", "event-sticker"]) assert.ok(bar.includes(`id="${id}"`), id);
  for (const mark of ['data-wrap="**"', 'data-wrap="*"', 'data-wrap="~~"']) assert.ok(html.includes(mark), mark);
  assert.match(line(`${T} #event-rich .rich-bar > button.on`), /background: var\(--pw-tint\); color: var\(--pw-lit\);/);
});

test("GIFs and stickers in a description sit on their own lines and show as pictures under the words", () => {
  assert.match(app, /function splitEventDescription\(text\)/);
  assert.match(app, /if \(isEventMediaLine\(line\)\) media\.push\(line\.trim\(\)\);/);
  assert.match(app, /const MAX_EVENT_MEDIA = 4;/);
  assert.match(app, /const MAX_EVENT_DESCRIPTION_BYTES = 1500;/);
  assert.match(app, /description: eventDescriptionDraft\(\),/);
});

test("the mic, headphones and Share Screen keys carry no lamp dots", () => {
  assert.match(css, /\.vp-voice :is\(#mute-btn, #deafen-btn, #vp-camera, \.vp-hand-key, #vp-more\)::after,\n[^{]*\.vp-activities\.vp-share::after \{ content: none !important; display: none !important; \}/);
});

test("floating panels lift off the pane with their own fill, a hairline and a soft shadow", () => {
  assert.match(css, /--float: #212856;/);
  assert.match(css, /--float: #ffffff;/);
  assert.match(css, /:is\(\.soundboard-pop, \.audio-pop, \.voice-pop, \.vr-pop, \.emoji-picker, \.emoji-pop, \.gif-picker, \.search-pop, \.pins-pop, \.pop-menu, \.profile-pop, \.text-style-pop\):not\(\.pt-tray\) \{ background: var\(--float\) !important; box-shadow: 0 0 0 1px var\(--float-line\), var\(--shadow-2\) !important; \}/);
});
