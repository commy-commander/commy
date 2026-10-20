import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const i18n = require("../scripts/i18n.js");
const { DEFAULT_LANGUAGE, registerLocale, availableLanguages, hasLanguage, setLanguage, interpolate, t, missingKeys } = i18n;

const ui = join(import.meta.dirname, "..");
const localesDir = join(ui, "scripts", "locales");
const localeFiles = readdirSync(localesDir).filter(f => f.endsWith(".js")).sort();
const locales = Object.fromEntries(localeFiles.map(f => [f.replace(/\.js$/, ""), require(join(localesDir, f))]));
for (const [code, strings] of Object.entries(locales)) registerLocale(code, strings);

const html = readFileSync(join(ui, "index.html"), "utf8");
const ENTITIES = { amp: "&", lt: "<", gt: ">", quot: '"' };
const decode = text => text.replace(/&(amp|lt|gt|quot);/g, (_, name) => ENTITIES[name]);

test("English is the default and every shipped locale names itself", () => {
  assert.equal(DEFAULT_LANGUAGE, "en");
  assert.ok(hasLanguage("en") && hasLanguage("es"));
  assert.deepEqual(availableLanguages().map(l => l.code).sort(), Object.keys(locales).sort());
  for (const { label } of availableLanguages()) assert.ok(label.length > 0);
});

test("t falls back to English, then to the key itself", () => {
  setLanguage("es");
  assert.equal(t("settings.tab.appearance"), "Apariencia");
  registerLocale("es-only-test", { "language.name": "Test" });
  setLanguage("es-only-test");
  assert.equal(t("settings.tab.appearance"), "Appearance");
  assert.equal(t("nothing.here"), "nothing.here");
  assert.equal(t("nothing.here", { a: 1 }), "nothing.here");
  setLanguage("en");
});

test("an unknown language falls back to English", () => {
  assert.equal(setLanguage("xx"), "en");
  assert.equal(t("settings.tab.updates"), "Updates");
});

test("interpolation fills named placeholders and leaves unknown ones visible", () => {
  assert.equal(interpolate("{host} (test server)", { host: "auth.example" }), "auth.example (test server)");
  assert.equal(interpolate("{a} and {b}", { a: "x" }), "x and {b}");
  assert.equal(interpolate("{a}", { a: 0 }), "0");
  assert.equal(interpolate("{a}", { a: null }), "{a}");
  assert.equal(t("signin.server.test", { host: "auth.example" }, "es"), "auth.example (servidor de pruebas)");
});

test("every shipped language covers every English key and adds none of its own", () => {
  for (const code of Object.keys(locales)) {
    assert.deepEqual(missingKeys(code), [], `${code} is missing keys`);
    assert.deepEqual(missingKeys("en", code), [], `${code} has keys English lacks`);
  }
});

test("every data-i18n key in the page exists in English and its markup text matches the English string", () => {
  const used = [...html.matchAll(/<([a-z0-9]+)[^>]*\sdata-i18n="([^"]+)"[^>]*>([^<]*)</g)];
  assert.ok(used.length > 40);
  for (const [, , key, text] of used) {
    assert.ok(Object.hasOwn(locales.en, key), `missing English for ${key}`);
    assert.equal(decode(text.trim()), locales.en[key], `markup for ${key} drifted from the dictionary`);
  }
  const titles = [...html.matchAll(/title="([^"]*)" data-i18n-title="([^"]+)"/g)];
  assert.ok(titles.length >= 6);
  for (const [, title, key] of titles) assert.equal(locales.en[key], title, `title for ${key} drifted from the dictionary`);
});

test("the page loads i18n before the locales and both before the app", () => {
  const order = [...html.matchAll(/<script src="scripts\/([^"]+)"><\/script>/g)].map(m => m[1]);
  const at = name => order.indexOf(name);
  assert.ok(at("i18n.js") >= 0);
  for (const f of localeFiles) assert.ok(at(`locales/${f}`) > at("i18n.js"), `${f} loads before i18n.js`);
  assert.ok(at("accessibility.js") > at("i18n.js") && at("app.js") > at("accessibility.js"));
  for (const f of localeFiles) assert.ok(at(`locales/${f}`) < at("app.js"));
});

test("the settings language picker lists the languages that were loaded", () => {
  const app = readFileSync(join(ui, "scripts", "app.js"), "utf8");
  assert.match(app, /availableLanguages\(\)\.map\(l => new Option\(l\.label, l\.code\)\)/);
});
