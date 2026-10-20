import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  ACCESSIBILITY_KEY, THEMES, SATURATION_MAX, ACCESSIBILITY_DEFAULTS, clampSaturation, readAccessibility, writeAccessibility,
  resolveAccessibility, accessibilityAttributes, saturationCss,
} = require("../scripts/accessibility.js");

const ui = join(import.meta.dirname, "..");
const read = (...path) => readFileSync(join(ui, ...path), "utf8");

test("defaults keep the dark look, follow the system for motion and start in English", () => {
  assert.equal(ACCESSIBILITY_KEY, "commy.accessibility");
  assert.deepEqual(ACCESSIBILITY_DEFAULTS, { reducedMotion: null, saturation: 100, highContrast: false, theme: "dark", language: "en" });
  assert.deepEqual(readAccessibility(null), ACCESSIBILITY_DEFAULTS);
  assert.deepEqual(readAccessibility(""), ACCESSIBILITY_DEFAULTS);
  assert.deepEqual(readAccessibility("not json"), ACCESSIBILITY_DEFAULTS);
  assert.deepEqual(readAccessibility("[1,2]"), ACCESSIBILITY_DEFAULTS);
});

test("reading tolerates bad values and unknown languages", () => {
  const prefs = readAccessibility(JSON.stringify({ reducedMotion: "yes", saturation: 500, highContrast: 1, theme: "sepia", language: "xx" }), ["en", "es"]);
  assert.deepEqual(prefs, ACCESSIBILITY_DEFAULTS);
  assert.equal(readAccessibility(JSON.stringify({ language: "es" }), ["en", "es"]).language, "es");
  assert.equal(readAccessibility(JSON.stringify({ language: "es" }), ["en"]).language, "en");
  assert.equal(clampSaturation(-5), 0);
  assert.equal(clampSaturation(42.6), 43);
  assert.equal(clampSaturation("abc"), SATURATION_MAX);
});

test("the persisted shape round-trips and only holds the five settings", () => {
  const prefs = { reducedMotion: true, saturation: 40, highContrast: true, theme: "light", language: "es", stray: 1 };
  const stored = JSON.parse(writeAccessibility(prefs));
  assert.deepEqual(stored, { reducedMotion: true, saturation: 40, highContrast: true, theme: "light", language: "es" });
  assert.deepEqual(readAccessibility(JSON.stringify(stored), ["en", "es"]), stored);
  assert.equal(JSON.parse(writeAccessibility({ ...ACCESSIBILITY_DEFAULTS, reducedMotion: false })).reducedMotion, false);
  assert.equal(JSON.parse(writeAccessibility(ACCESSIBILITY_DEFAULTS)).reducedMotion, null);
});

test("reduced motion follows the operating system until the switch is set either way", () => {
  const follow = { ...ACCESSIBILITY_DEFAULTS };
  assert.equal(resolveAccessibility(follow, { reducedMotion: true }).reducedMotion, true);
  assert.equal(resolveAccessibility(follow, { reducedMotion: false }).reducedMotion, false);
  assert.equal(resolveAccessibility(follow, {}).followsOsMotion, true);
  assert.equal(resolveAccessibility({ ...follow, reducedMotion: false }, { reducedMotion: true }).reducedMotion, false);
  assert.equal(resolveAccessibility({ ...follow, reducedMotion: true }, { reducedMotion: false }).reducedMotion, true);
  assert.equal(resolveAccessibility({ ...follow, reducedMotion: true }, {}).followsOsMotion, false);
});

test("the theme resolves to dark or light, following the system only when asked", () => {
  assert.deepEqual(THEMES, ["dark", "light", "system"]);
  const base = { ...ACCESSIBILITY_DEFAULTS };
  assert.equal(resolveAccessibility({ ...base, theme: "system" }, { dark: false }).theme, "light");
  assert.equal(resolveAccessibility({ ...base, theme: "system" }, { dark: true }).theme, "dark");
  assert.equal(resolveAccessibility({ ...base, theme: "system" }, {}).theme, "dark");
  assert.equal(resolveAccessibility({ ...base, theme: "light" }, { dark: true }).theme, "light");
  assert.equal(resolveAccessibility({ ...base, theme: "dark" }, { dark: false }).theme, "dark");
});

test("html attributes carry exactly what the stylesheet keys on", () => {
  const quiet = resolveAccessibility({ ...ACCESSIBILITY_DEFAULTS, reducedMotion: true, highContrast: true, theme: "light", saturation: 30 }, {});
  assert.deepEqual(accessibilityAttributes(quiet), { "data-theme": "light", "data-reduced-motion": "", "data-high-contrast": "", "data-saturation": "30" });
  assert.equal(saturationCss(quiet), "30%");
  const plain = resolveAccessibility(ACCESSIBILITY_DEFAULTS, {});
  assert.deepEqual(accessibilityAttributes(plain), { "data-theme": "dark", "data-reduced-motion": null, "data-high-contrast": null, "data-saturation": null });
  assert.equal(saturationCss(plain), "100%");
  const css = read("styles", "styles.css");
  for (const rule of ['html[data-theme="light"] {', "html[data-high-contrast] {", "html[data-reduced-motion] {", "html[data-saturation] .role-ink"]) {
    assert.ok(css.includes(rule), `stylesheet lacks ${rule}`);
  }
  assert.match(css, /html\[data-reduced-motion\] \*[\s\S]*?transition-duration: 0\.01ms !important/);
  assert.match(css, /--saturation: 100%;/);
  const overlay = read("overlay.html");
  assert.ok(overlay.includes('html[data-theme="light"]') && overlay.includes("html[data-reduced-motion]"));
  assert.ok(overlay.includes('<script src="scripts/accessibility.js"></script>'));
});

test("the settings page offers the three themes and the controls the app wires", () => {
  const html = read("index.html");
  assert.match(html, /<button class="settings-tab" data-tab="accessibility"/);
  const select = /<select id="theme-select"[\s\S]*?<\/select>/.exec(html)[0];
  const values = [...select.matchAll(/<option value="([^"]+)"/g)].map(m => m[1]);
  assert.deepEqual(values, THEMES);
  for (const id of ["reduced-motion-toggle", "reduced-motion-hint", "saturation", "saturation-label", "high-contrast-toggle", "language-select"]) {
    assert.ok(html.includes(`id="${id}"`), `page lacks #${id}`);
  }
  assert.match(html, /<input type="range" id="saturation" min="0" max="100"/);
});

test("the light theme keeps red buttons with white text and never leaves raw white on neutral active rows", () => {
  const css = read("styles", "styles.css");
  const light = /html\[data-theme="light"\] \{([\s\S]*?)\}/.exec(css)[1];
  assert.doesNotMatch(light, /--accent:/);
  assert.doesNotMatch(light, /--accent-ink:/);
  assert.match(light, /--header-primary: #17140f/);
  assert.doesNotMatch(css, /\.channel\.active \{[^}]*#fff/);
  assert.doesNotMatch(css, /\.segmented button\.active \{[^}]*#fff/);
  assert.doesNotMatch(css, /\.settings-tab\.active \{[^}]*#fff/);
});

test("role colours are painted through one helper so the saturation filter can find them", () => {
  const app = read("scripts", "app.js");
  assert.match(app, /function paintRole\(node, color\)/);
  assert.doesNotMatch(app, /\.style\.color = `#\$\{(?:member|role|person)\.color\}`/);
  assert.doesNotMatch(app, /\.style\.color = (?:member|person)\.color \?/);
});
