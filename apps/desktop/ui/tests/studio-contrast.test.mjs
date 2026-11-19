import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "studio-contrast.css");
const html = read("index.html");
const oldSheet = read("styles", "styles.css");
const HIGH_CONTRAST = 'html[data-look="studio"][data-high-contrast]';
const DARK = `${HIGH_CONTRAST}:not([data-theme="light"])`;
const LIGHT = `${HIGH_CONTRAST}[data-theme="light"]`;

function rules(sheet) {
  return sheet.split("}").map(block => block.split("{")).filter(([head]) => head && head.trim()).map(([head, body]) => ({
    selectors: head.split(",").map(s => s.trim()),
    body: body || "",
  }));
}

function tokens(selector) {
  const rule = rules(css).find(r => r.selectors.includes(selector));
  assert.ok(rule, selector);
  return Object.fromEntries([...rule.body.matchAll(/(--[a-z0-9-]+):\s*([^;]+);/g)].map(m => [m[1], m[2].trim()]));
}

function luminance(hex) {
  const value = hex.replace("#", "");
  return [0, 2, 4]
    .map(i => parseInt(value.slice(i, i + 2), 16) / 255)
    .map(c => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4))
    .reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i], 0);
}

function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const SURFACES = { dark: ["#050608", "#0d0f14", "#12141a", "#20242d", "#2c313d"], light: ["#ffffff", "#f4f5f8", "#f1f2f5", "#e6e9ef"] };

test("the high contrast sheet loads with the new look, after every other sheet", () => {
  assert.match(html, /<link rel="stylesheet" id="studio-contrast" href="styles\/studio-contrast\.css" media="not all" \/>/);
  const links = [...html.matchAll(/<link rel="stylesheet" id="([a-z-]+)"/g)].map(m => m[1]);
  assert.equal(links.at(-1), "studio-contrast");
});

test("every rule applies only while high contrast is on, with no comments", () => {
  for (const { selectors } of rules(css)) {
    for (const selector of selectors) assert.ok(selector.startsWith(HIGH_CONTRAST), selector);
  }
  assert.equal(css.includes("/*"), false);
});

test("no text field gets a square box of its own", () => {
  for (const sheet of [css, oldSheet]) {
    for (const { selectors, body } of rules(sheet)) {
      const boxesField = selectors.some(s => s.includes("data-high-contrast") && /(^|[\s,(>])(textarea|input|select)\b(?!::placeholder)/.test(s));
      if (boxesField) assert.equal(/box-shadow|outline|border/.test(body), false, selectors.join(", "));
    }
  }
});

test("text is body-text strong and edges reach 3 to 1 on every surface, in both themes", () => {
  for (const [theme, selector] of [["dark", DARK], ["light", LIGHT]]) {
    const t = tokens(selector);
    for (const surface of SURFACES[theme]) {
      for (const name of ["--header-primary", "--text-normal", "--text-muted", "--text-faint", "--tab-label", "--link"]) {
        assert.ok(contrast(t[name], surface) >= 4.5, `${theme} ${name} on ${surface}`);
      }
      assert.ok(contrast(t["--hc-edge"], surface) >= 3, `${theme} edge on ${surface}`);
    }
  }
});

test("keys, the chat bar and inputs all draw a full edge instead of glass", () => {
  for (const selector of [DARK, LIGHT]) {
    const t = tokens(selector);
    assert.match(t["--slot"], /inset 0 0 0 1px var\(--hc-edge\)/);
    assert.match(t["--bar-field-shadow"], /inset 0 0 0 1px var\(--hc-edge\)/);
    assert.match(t["--k-mute-shadow"], /inset 0 0 0 1px var\(--hc-edge\)/);
    assert.equal(t["--fx-blur"], "0px");
  }
  const dark = tokens(DARK);
  assert.equal(dark["--glass-fx"], "none");
  assert.match(dark["--glass-rim"], /inset 0 0 0 1px var\(--hc-edge\)/);
  assert.match(dark["--k-plain-shadow"], /inset 0 0 0 1px var\(--hc-edge\)/);
});
