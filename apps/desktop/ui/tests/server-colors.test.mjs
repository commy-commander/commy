import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { SERVER_COLORS, serverColors, serverTint } = require("../scripts/server-colors.js");

function luminance(hex) {
  const [r, g, b] = [1, 3, 5].map(i => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function hue(hex) {
  const [r, g, b] = [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16) / 255);
  const max = Math.max(r, g, b);
  const d = max - Math.min(r, g, b);
  if (d === 0) return 0;
  const h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return (h * 60 + 360) % 360;
}

test("a server always gets the same colours", () => {
  assert.deepEqual(serverColors("s1"), serverColors("s1"));
  assert.equal(serverTint("s1"), serverColors("s1").from);
  assert.match(serverColors(undefined).css, /^linear-gradient/);
});

function lightnessAndSaturation(hex) {
  const [r, g, b] = [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16) / 255);
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const s = max === min ? 0 : (max - min) / (1 - Math.abs(2 * l - 1));
  return { l, s };
}

test("every server colour carries white text, stays vivid rather than muddy, and clears the accent red", () => {
  for (const pair of SERVER_COLORS) {
    for (const color of pair) {
      assert.ok(1.05 / (luminance(color) + 0.05) >= 4.5, `${color} too light for white text`);
      const { l, s } = lightnessAndSaturation(color);
      assert.ok(l >= 0.3, `${color} is too dark and looks muddy`);
      assert.ok(s >= 0.4, `${color} is too grey`);
      const h = hue(color);
      assert.ok(h >= 170 && h <= 340, `${color} hue ${h} clashes with the red or yellow accent`);
    }
  }
});
