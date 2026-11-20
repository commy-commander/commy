import "./i18n-global.mjs";
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
const ENTITIES = { amp: "&", lt: "<", gt: ">", quot: '"', middot: "\u00b7", uarr: "\u2191", darr: "\u2193", nbsp: "\u00a0" };
const decode = text => text
  .replace(/&#(\d+);/g, (_, code) => String.fromCodePoint(Number(code)))
  .replace(/&([a-z]+);/g, (whole, name) => ENTITIES[name] ?? whole);
const flat = text => decode(text).replace(/\s+/g, " ").trim();
const SAME_IN_EVERY_LANGUAGE = new Set(["Commy", "GIPHY", "Tenor", "KLIPY", "YouTube", "Twitch", "Google", "Apple", "Ctrl", "Alt", "Shift", "OK"]);
const NEEDS_WORDS = /[A-Za-z]{2}/;
const VOID_TAGS = new Set(["input", "img", "br", "hr", "meta", "link", "source", "area", "col", "embed", "wbr", "track", "param", "base"]);
const UNTRANSLATED_TAGS = new Set(["script", "style", "svg", "template", "title", "code", "kbd"]);
const TRANSLATED_ATTRIBUTES = ["title", "placeholder", "aria-label", "alt"];
const wantsWords = text => {
  const shown = flat(text);
  return NEEDS_WORDS.test(shown) && !SAME_IN_EVERY_LANGUAGE.has(shown);
};

function attributesOf(source) {
  const found = {};
  for (const [, name, quoted] of source.matchAll(/([^\s=>/]+)(?:\s*=\s*("[^"]*"|'[^']*'|[^\s>]+))?/g)) {
    found[name] = quoted === undefined ? "" : quoted.replace(/^["']|["']$/g, "");
  }
  return found;
}

function untaggedPageText(page) {
  const stack = [];
  const loose = [];
  const tags = /<!--[\s\S]*?-->|<(\/?)([a-zA-Z][\w-]*)((?:\s+[^\s=>/]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?)*)\s*(\/?)>/g;
  let at = 0;
  const skipping = () => stack.some(open => UNTRANSLATED_TAGS.has(open.tag) || open.attrs.translate === "no");
  const textBefore = end => {
    const text = page.slice(at, end);
    const parent = stack[stack.length - 1];
    if (parent && !skipping() && wantsWords(text) && !("data-i18n" in parent.attrs)) loose.push(flat(text));
  };
  for (const match of page.matchAll(tags)) {
    textBefore(match.index);
    at = match.index + match[0].length;
    if (match[0].startsWith("<!--")) continue;
    const [, closing, rawTag, rawAttrs, selfClosing] = match;
    const tag = rawTag.toLowerCase();
    if (closing) {
      const open = stack.map(entry => entry.tag).lastIndexOf(tag);
      if (open >= 0) stack.length = open;
      continue;
    }
    const attrs = attributesOf(rawAttrs);
    if (!skipping() && !UNTRANSLATED_TAGS.has(tag)) {
      for (const name of TRANSLATED_ATTRIBUTES) {
        if (attrs[name] && wantsWords(attrs[name]) && !(`data-i18n-${name}` in attrs)) loose.push(`${name}: ${flat(attrs[name])}`);
      }
    }
    if (VOID_TAGS.has(tag) || selfClosing) continue;
    if (tag === "script" || tag === "style") {
      const end = page.indexOf(`</${tag}`, at);
      at = page.indexOf(">", end) + 1;
      tags.lastIndex = at;
      continue;
    }
    stack.push({ tag, attrs });
  }
  return loose;
}

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
  assert.equal(t("signin.server.test", { host: "auth.example" }, "es"), "Servidor de pruebas de Commy (auth.example)");
});

test("every account server address problem the backend reports has a message", () => {
  for (const problem of ["https", "login", "ip", "domain", "ending"]) {
    const key = `signin.server.bad.${problem}`;
    for (const code of Object.keys(locales)) assert.notEqual(t(key, {}, code), key, `${code} lacks ${key}`);
  }
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
    assert.equal(flat(text), locales.en[key], `markup for ${key} drifted from the dictionary`);
  }
  for (const name of TRANSLATED_ATTRIBUTES) {
    const tagged = [...html.matchAll(new RegExp(`\\s${name}="([^"]*)" data-i18n-${name}="([^"]+)"`, "g"))];
    for (const [, value, key] of tagged) {
      assert.ok(Object.hasOwn(locales.en, key), `missing English for ${key}`);
      assert.equal(flat(value), locales.en[key], `${name} for ${key} drifted from the dictionary`);
    }
  }
  assert.equal([...html.matchAll(/data-i18n-([a-z-]+)="/g)].filter(([, name]) => !TRANSLATED_ATTRIBUTES.includes(name)).length, 0);
});

test("every piece of text a person sees on the page goes through the dictionary", () => {
  assert.deepEqual(untaggedPageText(html), []);
});

test("the untagged text finder catches loose text, loose labels and skips code and brand names", () => {
  const page = '<div><p>Hello there</p><p data-i18n="a">Fine</p><b title="Tip">x</b><code>npm run</code><span>Commy</span><svg><text>Logo</text></svg></div>';
  assert.deepEqual(untaggedPageText(page), ["Hello there", "title: Tip"]);
});

test("a retranslation only replaces text that still shows a dictionary string", () => {
  registerLocale("es", {});
  assert.equal(i18n.isShownTranslation("settings.tab.appearance", "Appearance"), true);
  assert.equal(i18n.isShownTranslation("settings.tab.appearance", "  Apariencia "), true);
  assert.equal(i18n.isShownTranslation("settings.tab.appearance", "Someone's own words"), false);
  assert.equal(i18n.isShownTranslation("settings.tab.appearance", ""), false);
});

test("the page loads i18n before the locales and both before the app", () => {
  const order = [...html.matchAll(/<script src="scripts\/([^"]+)"><\/script>/g)].map(m => m[1]);
  const at = name => order.indexOf(name);
  assert.ok(at("i18n.js") >= 0);
  for (const f of localeFiles) assert.ok(at(`locales/${f}`) > at("i18n.js"), `${f} loads before i18n.js`);
  assert.ok(at("accessibility.js") > at("i18n.js") && at("app.js") > at("accessibility.js"));
  for (const f of localeFiles) assert.ok(at(`locales/${f}`) < at("app.js"));
});

test("dates follow the app language, and English keeps the system's own format", () => {
  i18n.setLanguage("es");
  assert.equal(i18n.dateLocale(), "es");
  assert.equal(new Date(Date.UTC(2026, 10, 15, 12)).toLocaleDateString(i18n.dateLocale(), { weekday: "short", timeZone: "UTC" }), "dom");
  i18n.setLanguage("en");
  assert.deepEqual(i18n.dateLocale(), []);
});

test("changing the language reloads the app so every screen switches, but never drops a call", () => {
  const app = readFileSync(new URL("../scripts/app.js", import.meta.url), "utf8");
  assert.match(app, /\$\("language-select"\)\.addEventListener\("change", e => \{\n\s+prefs\.language = e\.target\.value;\n\s+saveAccessibility\(prefs\);\n\s+if \(state\.voiceChannelId \|\| state\.call\) toast\(t\("accessibility\.language\.later"\)\);\n\s+else window\.location\.reload\(\);/);
  for (const locale of Object.values(locales)) assert.ok(locale["accessibility.language.later"]);
});

test("the saved language is chosen before any other script builds words", () => {
  const order = [...html.matchAll(/<script src="scripts\/([^"]+)"><\/script>/g)].map(m => m[1]);
  const start = order.indexOf("i18n-start.js");
  assert.deepEqual(order.slice(0, start + 1), ["i18n.js", ...localeFiles.map(f => `locales/${f}`), "i18n-start.js"]);
  const { startingLanguage } = require("../scripts/i18n-start.js");
  assert.equal(startingLanguage('{"language":"es","theme":"dark"}'), "es");
  assert.equal(startingLanguage(null), "en");
  assert.equal(startingLanguage("not json"), "en");
  const source = readFileSync(new URL("../scripts/i18n-start.js", import.meta.url), "utf8");
  assert.ok(source.includes(`localStorage.getItem(${JSON.stringify(require("../scripts/accessibility.js").ACCESSIBILITY_KEY)})`));
});

test("the settings language picker lists the languages that were loaded", () => {
  const app = readFileSync(join(ui, "scripts", "app.js"), "utf8");
  assert.match(app, /availableLanguages\(\)\.map\(l => new Option\(l\.label, l\.code\)\)/);
});

test("on a Mac the tray settings say menu bar, in every language, and other systems keep the tray", () => {
  const trayKeys = Object.keys(locales.en).filter(key => /tray|bandeja/i.test(locales.en[key]) || /tray/i.test(key));
  assert.ok(trayKeys.length >= 9);
  for (const code of Object.keys(locales)) {
    i18n.setLanguage(code);
    i18n.setPlatform("mac");
    for (const key of trayKeys.filter(key => !key.endsWith(".mac"))) {
      assert.doesNotMatch(t(key), /tray|bandeja/i, `${code} ${key} still names the tray on a Mac`);
      assert.match(t(key), /menu bar|barra de menús/i, `${code} ${key} should name the menu bar on a Mac`);
    }
    i18n.setPlatform(null);
    assert.doesNotMatch(t("ui.keep-running-in-the-tray"), /menu bar|barra de menús/i);
  }
  i18n.setLanguage("en");
});

test("a Mac wording still counts as the shown translation so a later pass keeps it", () => {
  i18n.setPlatform("mac");
  assert.equal(i18n.isShownTranslation("ui.keep-running-in-the-tray", "Keep running in the menu bar"), true);
  assert.equal(i18n.isShownTranslation("ui.keep-running-in-the-tray", "Keep running in the tray"), true);
  i18n.setPlatform(null);
  assert.equal(i18n.isShownTranslation("ui.keep-running-in-the-tray", "Keep running in the menu bar"), false);
});

test("only a Mac computer gets the Mac wording, not an iPad or other systems", () => {
  const { wordingPlatform } = require("../scripts/i18n-start.js");
  assert.equal(wordingPlatform({ platform: "MacIntel", maxTouchPoints: 0 }), "mac");
  assert.equal(wordingPlatform({ platform: "MacIntel", maxTouchPoints: 5 }), null);
  assert.equal(wordingPlatform({ platform: "Win32" }), null);
  assert.equal(wordingPlatform({ platform: "Linux x86_64" }), null);
  assert.equal(wordingPlatform({ platform: "iPhone", maxTouchPoints: 5 }), null);
  assert.equal(wordingPlatform(undefined), null);
});
