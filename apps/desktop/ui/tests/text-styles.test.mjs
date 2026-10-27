import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { TEXT_COLORS, TEXT_FONTS, TEXT_EFFECTS } = require("../scripts/markdown.js");

const ui = join(import.meta.dirname, "..");
const css = readFileSync(join(ui, "styles", "styles.css"), "utf8");
const app = readFileSync(join(ui, "scripts", "app.js"), "utf8");

function block(selector) {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, `${selector} is missing`);
  return css.slice(start, css.indexOf("\n}", start));
}

function vars(selector) {
  return Object.fromEntries([...block(selector).matchAll(/--([a-z-]+):\s*([^;]+);/g)].map(m => [m[1], m[2].trim()]));
}

function hexRgb(hex) {
  const n = parseInt(hex.slice(1), 16);
  return [n >> 16, (n >> 8) & 255, n & 255];
}

function hslRgb(h, s, l) {
  const k = n => (n + h / 30) % 12;
  const a = s * Math.min(l, 1 - l);
  const f = n => l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
  return [f(0), f(8), f(4)].map(v => Math.round(v * 255));
}

function luminance(rgb) {
  const [r, g, b] = rgb.map(v => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const themes = { dark: { ...vars(":root") }, light: { ...vars(":root"), ...vars('html[data-theme="light"]') } };

test("every text colour stays readable on chat and panel backgrounds in both themes", () => {
  for (const [theme, v] of Object.entries(themes)) {
    for (const bg of ["bg-mid", "bg-raised"]) {
      for (const name of TEXT_COLORS) {
        const ratio = contrast(hexRgb(v[`tx-${name}`]), hexRgb(v[bg]));
        assert.ok(ratio >= 4.5, `${name} on ${bg} in ${theme} is ${ratio.toFixed(2)}`);
      }
    }
  }
});

test("rainbow letters stay readable at every hue in both themes", () => {
  for (const [theme, v] of Object.entries(themes)) {
    const light = parseFloat(v["tx-rainbow-light"]) / 100;
    for (const bg of ["bg-mid", "bg-raised"]) {
      for (let i = 0; i < 14; i++) {
        const ratio = contrast(hslRgb((i * 26) % 360, 0.85, light), hexRgb(v[bg]));
        assert.ok(ratio >= 4.5, `rainbow letter ${i} on ${bg} in ${theme} is ${ratio.toFixed(2)}`);
      }
    }
  }
});

test("every style name has its look, a label in the picker, and a still version", () => {
  for (const name of TEXT_COLORS) {
    assert.match(css, new RegExp(`\\.tx-${name} \\{ color: var\\(--tx-${name}\\); \\}`));
    assert.match(css, new RegExp(`\\.tx-swatch-${name} \\{`));
  }
  for (const name of TEXT_FONTS) assert.match(css, new RegExp(`\\.tx-font-${name} \\{`));
  for (const name of TEXT_EFFECTS) assert.match(css, new RegExp(`\\.tx-${name}[ .{]`));
  for (const name of [...TEXT_COLORS, ...TEXT_FONTS, ...TEXT_EFFECTS]) assert.match(app, new RegExp(`\\b${name}: "[A-Z]`));
  for (const moving of ["wave", "shimmer", "shake"]) {
    assert.match(css, new RegExp(`html\\[data-reduced-motion\\][^{]*\\.tx-${moving}`));
  }
});
