import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { SETTINGS_PAGE_ICONS, SETTINGS_PICK_ICONS, SETTINGS_FILTER_ICONS, METER_BARS, settingsPageIcon, settingsPaneGroups, meterBarClass, meterLit } = require("../scripts/studio-settings.js");
const { VOICE_FILTERS } = require("../scripts/voice-filters.js");
const { STUDIO_ICON_PATHS } = require("../vendor/phosphor/icons.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio-settings.css");
const app = read("../scripts/app.js");
const en = read("../scripts/locales/en.js");
const es = read("../scripts/locales/es.js");

const SWITCH = 'html[data-look="studio"]';
const ROOMY = 'html[data-look="studio"]:not([data-fit="slim"])';
const splitTop = list => {
  const parts = [];
  let depth = 0;
  let from = 0;
  [...list].forEach((ch, at) => {
    if (ch === "(") depth += 1;
    if (ch === ")") depth -= 1;
    if (ch === "," && depth === 0) {
      parts.push(list.slice(from, at));
      from = at + 1;
    }
  });
  parts.push(list.slice(from));
  return parts.map(part => part.trim()).filter(Boolean);
};
const selectors = sheet => [...sheet.matchAll(/(^|\})\s*([^{}@]+)\{/g)].flatMap(match => splitTop(match[2]));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("the settings window's new look stays off until the switch is on", () => {
  const all = selectors(css);
  assert.ok(all.length > 60);
  for (const selector of all) assert.ok(selector.startsWith(SWITCH), selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  const sheet = html.indexOf('<link rel="stylesheet" id="studio-settings" href="styles/studio-settings.css" media="not all" />');
  assert.ok(sheet > html.indexOf('id="studio-profile"'));
  assert.ok(sheet < html.indexOf('id="studio-phone"'));
  const script = html.indexOf('<script src="scripts/studio-settings.js"></script>');
  assert.ok(script > html.indexOf('<script src="scripts/studio-icons.js"></script>'));
  assert.ok(script < html.indexOf('<script src="scripts/app.js"></script>'));
  assert.match(app, /setupSidebarResize\(\);\n  if \(studioLook\(\)\) studioSettingsShell\(closeSettingsLayers\);/);
});

test("Slim keeps the phone's settings: the panes, picks and icons step aside", () => {
  assert.match(css, /html\[data-look="studio"\]\[data-fit="slim"\] \.set-pane,\nhtml\[data-look="studio"\]\[data-fit="slim"\] \.set-pick \{ display: contents; \}/);
  for (const part of [".set-pick > .i", ".settings-tab > .i", "#user-settings .settings-nav-title"]) assert.ok(css.includes(`html[data-look="studio"][data-fit="slim"] ${part}`), part);
  for (const selector of selectors(css).filter(s => !s.includes('[data-fit="slim"]') && !s.startsWith('html[data-look="studio"] .user-actions'))) {
    assert.ok(selector.includes(':not([data-fit="slim"])'), selector);
  }
});

test("every page in the three lists has an icon from the app's set", () => {
  for (const [layer, icons] of Object.entries(SETTINGS_PAGE_ICONS)) {
    for (const name of Object.values(icons)) assert.ok(STUDIO_ICON_PATHS[name], `${layer} ${name}`);
  }
  for (const name of Object.values(SETTINGS_PICK_ICONS)) assert.ok(STUDIO_ICON_PATHS[name], name);
  for (const id of ["user-settings", "server-settings", "channel-settings"]) {
    const start = html.indexOf(`id="${id}"`);
    const nav = html.slice(start, html.indexOf("</nav>", start));
    for (const tab of nav.matchAll(/<button class="settings-tab[^"]*"([^>]*)>/g)) {
      const key = (tab[1].match(/data-tab="([^"]+)"/) || tab[1].match(/id="([^"]+)"/))[1];
      assert.ok(settingsPageIcon(id, key), `${id} ${key}`);
    }
  }
  assert.equal(settingsPageIcon("user-settings", "voice"), "mic");
  assert.equal(settingsPageIcon("server-settings", "overview"), "info");
  assert.equal(settingsPageIcon("user-settings", "constructor"), "");
  assert.equal(settingsPageIcon("server-settings", "file-storage"), "folder");
});

test("a page's sections become cards: a heading starts one, the title and intro stay out", () => {
  assert.deepEqual(settingsPaneGroups(["title", "title", "head", "part", "part", "head", "part"]), [[2, 3, 4], [5, 6]]);
  assert.deepEqual(settingsPaneGroups(["title", "part", "part", "head", "part"]), [[1, 2], [3, 4]]);
  assert.deepEqual(settingsPaneGroups(["title"]), []);
});

test("the window keeps the chat window's place, list and lit pill", () => {
  assert.match(rule(`${ROOMY} .settings-layer`), /inset: calc\(var\(--st-mt\) \* 2 \+ 52px\) var\(--st-mr\) var\(--st-mb\) var\(--st-ml\);/);
  assert.match(rule(`${ROOMY} .settings-nav`), /flex: 0 0 var\(--set-side, 248px\);/);
  assert.match(rule(`${ROOMY} .settings-nav`), /box-shadow: inset -1px 0 0 var\(--line\);/);
  assert.match(rule(`${ROOMY} .settings-nav-title`), /font: 700 21px\/24px var\(--font-display\);/);
  assert.match(rule(`${ROOMY} .settings-search`), /height: 36px;\n  padding: 0 14px 0 41px;/);
  assert.match(rule(`${ROOMY} .settings-nav-group`), /padding: 9px 10px 4px;\n  color: var\(--text-muted\);\n  font: 600 13px\/16px var\(--font\);/);
  assert.match(rule(`${ROOMY} .settings-tab`), /min-height: 26px;\n  padding: 0 10px;/);
  assert.match(rule(`${ROOMY} .settings-tab > .i`), /width: 17px;\n  height: 17px;/);
  const lit = rule(`${ROOMY} .settings-tab.active,\n${ROOMY} .settings-tab.active:hover`);
  assert.match(lit, /background: var\(--pillow-top\), var\(--accent\);/);
  assert.match(lit, /margin: -2px -10px -2px -2px;\n  padding: 0 14px 0 12px;/);
  assert.match(css, /body:has\(\.settings-layer:not\(\.hidden\)\) \.app > :not\(\.rail\),/);
});

test("Done takes the gear's place while a settings window is open, and Esc still closes", () => {
  assert.match(css, /html\[data-look="studio"\] \.user-actions > \.set-done \{ display: none; \}/);
  assert.match(css, /body:has\(\.settings-layer:not\(\.hidden\)\) \.user-actions > \.set-done \{ display: inline-flex; \}/);
  assert.match(css, /body:has\(\.settings-layer:not\(\.hidden\)\) \.user-panel \{ padding-right: calc\(71px - 25px \* var\(--st-hang\)\); \}/);
  assert.match(rule(`${ROOMY} .settings-close-wrap`), /display: none;/);
  const shell = read("../scripts/studio-settings.js");
  assert.match(shell, /done\.className = "key sm cream set-done";/);
  assert.match(en, /"settings\.done": "Done",/);
  assert.match(es, /"settings\.done": "Listo",/);
});

test("the page is one centred column of the guide's cards", () => {
  assert.match(rule(`${ROOMY} .settings-content`), /padding: 26px var\(--st-gap\) 40px;/);
  assert.match(rule(`${ROOMY} .settings-content-inner`), /max-width: 760px;\n  margin: 0 auto;/);
  assert.match(rule(`${ROOMY} .settings-page > h1`), /font: 700 28px\/32px var\(--font-display\);/);
  assert.match(rule(`${ROOMY} .settings-page > .settings-intro`), /font: 400 14px\/1\.3 var\(--font\);/);
  assert.match(rule(`${ROOMY} .set-pane`), /padding: 0 22px 18px;\n  border-radius: var\(--radius-lg\);/);
  assert.match(rule(`${ROOMY} .set-pane > .settings-subhead`), /height: 52px;\n  margin: 0;\n  padding-top: 19px;/);
  assert.match(rule(`${ROOMY} .set-pane > .settings-subhead`), /font: 700 17px\/20px var\(--font-display\);/);
});

test("fields, picks, sliders, switches and keys are the guide's parts", () => {
  assert.match(css, /:where\(input\[type="text"\][^{]+\{\n  height: 46px;\n  padding: 0 18px;\n  border: 0;\n  border-radius: var\(--radius-pill\);/);
  assert.match(css, /box-shadow: var\(--slot\);/);
  assert.match(rule(`${ROOMY} .settings-layer .slider-row`), /grid-template-columns: minmax\(112px, max-content\) minmax\(0, 1fr\) 48px;/);
  assert.match(rule(`${ROOMY} .settings-layer input[type="range"]::-webkit-slider-thumb`), /width: 22px;\n  height: 22px;/);
  assert.match(rule(`${ROOMY} .settings-layer input.switch`), /width: 48px;\n  height: 28px;/);
  assert.match(rule(`${ROOMY} .settings-layer input.switch:checked::after`), /translate: 20px 0;/);
  assert.match(rule(`${ROOMY} .settings-layer .segmented > button:not(.key).active`), /box-shadow: var\(--pillow-sm\);/);
  assert.match(rule(`${ROOMY} .settings-layer .btn-danger`), /--k-ink: var\(--danger-text\);/);
  assert.match(rule(`${ROOMY} .settings-layer .btn-small`), /height: 32px;/);
});

test("in the list only Leave server is red, and your own settings ring no server on the strip", () => {
  assert.equal(css.includes(".settings-tab.danger"), false);
  assert.match(rule(`${ROOMY} #leave-server-btn,\n${ROOMY} #leave-server-btn > .i,\n${ROOMY} #leave-server-btn:hover`), /color: var\(--danger-text\);/);
  const ring = `${ROOMY} body:has(#user-settings:not(.hidden))`;
  assert.match(rule(`${ring} .rail-home.active`), /outline: 0;/);
  assert.ok(css.includes(`${ring} .server-icon.active { outline: 0; }`));
});

test("the Voice page: no extra lines, a 40-bar meter whose gate is the threshold, and the filters as latch keys", () => {
  assert.match(rule(`${ROOMY} #input-mode-note,\n${ROOMY} #sensitivity-label`), /display: none;/);
  assert.equal(METER_BARS, 40);
  assert.equal(meterLit(0), 0);
  assert.equal(meterLit(50), 20);
  assert.equal(meterLit(140), 40);
  assert.equal(meterLit(NaN), 0);
  assert.equal(meterBarClass(5, 20), "on");
  assert.equal(meterBarClass(30, 40), "on warm");
  assert.equal(meterBarClass(36, 40), "on hot");
  assert.equal(meterBarClass(20, 20), "");
  const meter = rule(`${ROOMY} .settings-layer .mic-meter`);
  for (const line of ["display: flex;", "gap: 3px;", "height: 30px;", "padding: 7px 13px;", "border-radius: var(--radius-pill);", "box-shadow: var(--slot);"]) assert.ok(meter.includes(line), line);
  assert.match(rule(`${ROOMY} .settings-layer #sensitivity-db`), /margin: -38px 11\.5px 0;/);
  assert.match(rule(`${ROOMY} .settings-layer #sensitivity-db::-webkit-slider-thumb`), /width: 3px;\n  height: 24px;/);
  assert.match(app, /if \(studioLook\(\)\) studioSettingsMeter\(listening \? meterPercent\(db\) : 0\);/);
  for (const filter of VOICE_FILTERS) assert.ok(STUDIO_ICON_PATHS[SETTINGS_FILTER_ICONS[filter.key]], filter.key);
  assert.match(rule(`${ROOMY} .settings-layer .segmented.voice-filters`), /grid-template-columns: repeat\(4, minmax\(0, 1fr\)\);\n  gap: 9px 8px;/);
  assert.match(rule(`${ROOMY} .settings-layer .segmented.voice-filters > .key`), /height: 40px;/);
});
