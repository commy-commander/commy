import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const { normalizeSearchText, searchSettings, settingsResultPlace, SETTINGS_SEARCH_LIMIT } = createRequire(import.meta.url)("../scripts/settings-search.js");

const entry = (page, pageTitle, kind, title, section = "", keywords = "") => ({ page, pageTitle, kind, title, section, keywords });
const ENTRIES = [
  entry("voice", "Voice", "page", "Voice", "", "audio microphone mic"),
  entry("voice", "Voice", "section", "Voice Processing"),
  entry("voice", "Voice", "setting", "Noise Cancellation", "Voice Processing", "background noise rnnoise"),
  entry("voice", "Voice", "setting", "Echo Cancellation", "Voice Processing", "feedback speakers"),
  entry("voice", "Voice", "setting", "Go AFK after", "Idle", "idle away"),
  entry("appearance", "Appearance", "page", "Appearance", "", "look theme dark light"),
  entry("appearance", "Appearance", "setting", "Theme", "", "dark mode light mode"),
  entry("appearance", "Appearance", "setting", "Language", "", "translate spanish espanol"),
  entry("sounds", "Sounds", "page", "Sounds"),
  entry("sounds", "Sounds", "setting", "Sound Volume", "App Sounds"),
  entry("sounds", "Sounds", "setting", "Soundboard Volume", "Soundboard"),
];
const titles = hits => hits.map(hit => hit.title);

test("normalizing drops accents, case and punctuation", () => {
  assert.equal(normalizeSearchText("  Privacy & Safety "), "privacy safety");
  assert.equal(normalizeSearchText("Cámara"), "camara");
  assert.equal(normalizeSearchText("Text-to-Speech"), "text to speech");
  assert.equal(normalizeSearchText(null), "");
});

test("an empty or blank query finds nothing", () => {
  assert.deepEqual(searchSettings(ENTRIES, ""), []);
  assert.deepEqual(searchSettings(ENTRIES, "   "), []);
  assert.deepEqual(searchSettings(ENTRIES, "&&"), []);
});

test("words match the start of words in titles and keywords", () => {
  assert.deepEqual(titles(searchSettings(ENTRIES, "noise")), ["Noise Cancellation"]);
  assert.deepEqual(titles(searchSettings(ENTRIES, "cancel")), ["Noise Cancellation", "Echo Cancellation"]);
  assert.deepEqual(titles(searchSettings(ENTRIES, "dark mode")), ["Theme"]);
  assert.deepEqual(titles(searchSettings(ENTRIES, "espanol")), ["Language"]);
  assert.deepEqual(titles(searchSettings(ENTRIES, "ancel")), []);
});

test("every word has to match somewhere", () => {
  assert.deepEqual(titles(searchSettings(ENTRIES, "noise echo")), []);
  assert.deepEqual(titles(searchSettings(ENTRIES, "echo speakers")), ["Echo Cancellation"]);
});

test("a page name alone finds the page, not every setting on it", () => {
  assert.deepEqual(titles(searchSettings(ENTRIES, "appearance")), ["Appearance"]);
});

test("a page name narrows a setting search to that page", () => {
  assert.deepEqual(titles(searchSettings(ENTRIES, "sounds volume")), ["Sound Volume", "Soundboard Volume"]);
  assert.deepEqual(titles(searchSettings(ENTRIES, "voice noise")), ["Noise Cancellation"]);
});

test("exact and leading title matches rank first, then page order", () => {
  assert.deepEqual(titles(searchSettings(ENTRIES, "sound")), ["Sounds", "Sound Volume", "Soundboard Volume"]);
  assert.deepEqual(titles(searchSettings(ENTRIES, "voice")), ["Voice", "Voice Processing"]);
});

test("the same title on one page is listed once", () => {
  const twice = [entry("voice", "Voice", "section", "Camera"), entry("voice", "Voice", "setting", "Camera", "Camera")];
  assert.equal(searchSettings(twice, "camera").length, 1);
});

test("results stop at the limit", () => {
  const many = Array.from({ length: 40 }, (_, i) => entry("p", "Page", "setting", `Volume ${i}`));
  assert.equal(searchSettings(many, "volume").length, SETTINGS_SEARCH_LIMIT);
  assert.equal(searchSettings(many, "volume", 3).length, 3);
});

test("a result names its page and section", () => {
  assert.equal(settingsResultPlace(ENTRIES[0]), "");
  assert.equal(settingsResultPlace(ENTRIES[2]), "Voice › Voice Processing");
  assert.equal(settingsResultPlace(ENTRIES[6]), "Appearance");
  assert.equal(settingsResultPlace(entry("voice", "Voice", "section", "Idle")), "Voice");
});

const html = readFileSync(join(import.meta.dirname, "..", "index.html"), "utf8");
const userSettings = html.slice(html.indexOf('id="user-settings"'), html.indexOf('id="channel-settings"'));
const tabs = [...userSettings.matchAll(/class="settings-tab[^"]*" data-tab="([^"]+)"/g)].map(m => m[1]);
const pages = [...userSettings.matchAll(/<section class="settings-page" data-page="([^"]+)"/g)].map(m => m[1]);

test("every user settings tab opens a page and every page has a tab, in the same order", () => {
  assert.deepEqual(tabs, pages);
  assert.equal(new Set(tabs).size, tabs.length);
});

test("every user settings page opens with a heading and a one-line intro", () => {
  for (const page of pages) {
    const at = userSettings.indexOf(`data-page="${page}"`);
    const opening = userSettings.slice(at, userSettings.indexOf("</section>", at));
    assert.match(opening, /^[^>]*>\s*<h1 data-i18n="[^"]+">[^<]+<\/h1>\s*<p class="settings-intro" data-i18n="settings\.intro\.[^"]+">/, `${page} opening`);
  }
});

test("the search box sits above the tabs and the page loads search before the app", () => {
  assert.ok(userSettings.indexOf('id="settings-search"') < userSettings.indexOf('class="settings-tab'));
  const order = [...html.matchAll(/<script src="scripts\/([^"]+)"><\/script>/g)].map(m => m[1]);
  assert.ok(order.indexOf("settings-search.js") >= 0 && order.indexOf("settings-search.js") < order.indexOf("app.js"));
});
