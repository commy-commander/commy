import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { SERVER_COLORS, STUDIO_SERVER_COLORS, serverColors, serverTint, serverTintIsLight } = require("../scripts/server-colors.js");

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

test("the new look gives a server one of eight colours by its id, and the old look keeps its own", () => {
  assert.equal(STUDIO_SERVER_COLORS.length, 8);
  assert.equal(new Set(STUDIO_SERVER_COLORS).size, 8);
  assert.equal(serverTint("s1", "classic"), serverColors("s1").from);
  assert.equal(serverTint("s1"), serverColors("s1").from);
  const seen = new Set();
  for (let n = 0; n < 200; n++) {
    const tint = serverTint(`server-${n}`, "studio");
    assert.ok(STUDIO_SERVER_COLORS.includes(tint));
    assert.equal(serverTint(`server-${n}`, "studio"), tint);
    seen.add(tint);
  }
  assert.equal(seen.size, 8);
});

test("only the ultramarine tile keeps white initials when it is the server you are in", () => {
  assert.deepEqual(STUDIO_SERVER_COLORS.map(serverTintIsLight), [false, true, true, true, true, true, true, true]);
  for (const pair of SERVER_COLORS) assert.equal(serverTintIsLight(pair[0]), false);
  assert.equal(serverTintIsLight(""), false);
  assert.equal(serverTintIsLight("var(--accent)"), false);
});
