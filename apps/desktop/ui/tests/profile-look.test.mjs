import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  AVATAR_BORDERS, CARD_EFFECTS, STUDIO_ORNAMENTS, STUDIO_FRAME_PARTS, STUDIO_BLOOM, STUDIO_FROST, STUDIO_DIAL,
  isAvatarBorder, isCardEffect, ornamentPoints, studioFrame, studioFrameArt, studioEffect,
} = require("../scripts/profile-look.js");

const ui = join(import.meta.dirname, "..");
const css = readFileSync(join(ui, "styles", "studio-profile.css"), "utf8").replace(/\r\n/g, "\n");
const user = readFileSync(join(ui, "..", "..", "..", "crates", "commy-core", "src", "user.rs"), "utf8");

test("the app and the UI offer the same borders and effects", () => {
  const listed = name => JSON.parse(`[${new RegExp(`pub const ${name}: \\[&str; \\d+\\] = \\[([^\\]]*)\\]`).exec(user)[1].trim().replace(/,$/, "")}]`);
  assert.deepEqual(AVATAR_BORDERS.map(b => b.id), listed("AVATAR_BORDERS"));
  assert.deepEqual(CARD_EFFECTS.map(e => e.id), listed("CARD_EFFECTS"));
  assert.equal(new RegExp("pub const AVATAR_BORDERS: \\[&str; (\\d+)\\]").exec(user)[1], String(AVATAR_BORDERS.length));
  assert.equal(new RegExp("pub const CARD_EFFECTS: \\[&str; (\\d+)\\]").exec(user)[1], String(CARD_EFFECTS.length));
  assert.ok(isAvatarBorder("orbit") && isAvatarBorder("dial") && !isAvatarBorder("gold") && !isAvatarBorder(undefined));
  assert.ok(isCardEffect("snow") && isCardEffect("meteors") && !isCardEffect("fire"));
});

test("the first eight borders and four effects keep their ids, so a saved choice stays", () => {
  assert.deepEqual(AVATAR_BORDERS.slice(0, 8).map(b => b.id), ["ring", "double", "sunset", "ocean", "neon", "stars", "leaves", "orbit"]);
  assert.deepEqual(CARD_EFFECTS.slice(0, 4).map(e => e.id), ["sparkles", "snow", "aurora", "bubbles"]);
  assert.deepEqual(AVATAR_BORDERS.slice(8).map(b => b.label), ["Halo", "Comet", "Eclipse", "Bloom", "Frost", "Ripple", "Prism", "Dial"]);
  assert.deepEqual(CARD_EFFECTS.slice(4).map(e => e.label), ["Fireflies", "Rain", "Meteors", "Petals", "Sunbeams", "Nebula", "Confetti"]);
  for (const list of [AVATAR_BORDERS, CARD_EFFECTS]) {
    assert.equal(new Set(list.map(item => item.id)).size, list.length);
    for (const item of list) assert.match(item.id, /^[a-z]+$/);
  }
});

test("every border and effect has a look, and the moving ones stop for reduced motion", () => {
  for (const { id } of AVATAR_BORDERS.slice(1)) assert.match(css, new RegExp(`\\.ab-${id}[ {.:,]`), `${id} border has no look`);
  for (const { id } of CARD_EFFECTS) assert.match(css, new RegExp(`\\.fx\\.${id}[ {.]`), `${id} effect has no look`);
  assert.ok(css.includes('html[data-look="studio"][data-reduced-motion] .avatar > .avatar-border,\nhtml[data-look="studio"][data-reduced-motion] .avatar > .avatar-border::after,\nhtml[data-look="studio"][data-reduced-motion] .avatar-border i,\nhtml[data-look="studio"][data-reduced-motion] .avatar-border .ab-art * { animation: none !important; }'));
  assert.ok(css.includes('html[data-look="studio"][data-reduced-motion] .card-effect.fx i,\nhtml[data-look="studio"][data-reduced-motion] .card-effect.fx g { animation: none !important; }'));
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{\n  html\[data-look="studio"\] \.avatar > \.avatar-border,\n  html\[data-look="studio"\] \.avatar > \.avatar-border::after,\n  html\[data-look="studio"\] \.avatar-border i,\n  html\[data-look="studio"\] \.avatar-border \.ab-art \* \{ animation: none !important; \}/);
});

test("bloom, frost and dial are each one drawing that scales with the avatar", () => {
  const drawn = ["bloom", "frost", "dial"];
  for (const { id } of AVATAR_BORDERS) {
    const art = studioFrameArt(id);
    assert.equal(art, studioFrameArt(id));
    assert.equal(art !== "", drawn.includes(id), id);
    assert.doesNotMatch(art, /style=|fill=|stroke=|#[0-9a-f]{3}/i, id);
    for (const [, names] of art.matchAll(/class="([^"]+)"/g)) assert.ok(css.includes(`.ab-art .${names.split(" ")[0]} `), `${id} ${names}`);
    for (const [, d] of art.matchAll(/ d="([^"]+)"/g)) {
      for (const n of d.match(/-?\d+(\.\d+)?/g).map(Number)) assert.ok(n >= -3 && n <= 103, `${id} ${n}`);
    }
  }
  assert.equal(studioFrameArt("none"), "");
  assert.equal(studioFrameArt(undefined), "");
  const count = (art, tag) => art.split(`<${tag} `).length - 1;
  const bloom = studioFrameArt("bloom");
  assert.equal(STUDIO_BLOOM.petals, 16);
  assert.equal(count(bloom, "path"), 48);
  assert.equal(count(bloom, "circle"), 17);
  assert.ok(bloom.startsWith('<g class="bl-wreath"><g class="bl-back">') && bloom.endsWith('<circle class="bl-seat" cx="50" cy="50" r="40.5" stroke-width="1.9"/>'));
  const frost = studioFrameArt("frost");
  assert.equal(STUDIO_FROST.points, 6);
  assert.deepEqual([...frost.matchAll(/class="([^"]+)"/g)].map(m => m[1]), ["fr-line", "fr-small", "fr-side", "fr-ring", "fr-edge", "fr-edge", "fr-tall", "fr-core"]);
  assert.equal(frost.match(/class="fr-side" d="([^"]+)"/)[1].split("Z").length - 1, 12);
  const dial = studioFrameArt("dial");
  assert.deepEqual([...dial.matchAll(/class="([^"]+)"/g)].map(m => m[1]), ["di-band", "di-rim", "di-seat", "di-minor", "di-hour", "di-quarter", "di-hand"]);
  const strokes = name => dial.match(new RegExp(`class="${name}" d="([^"]+)"`))[1].split("M").length - 1;
  assert.deepEqual([strokes("di-minor"), strokes("di-hour"), strokes("di-quarter")], [48, 8, 4]);
  assert.ok(STUDIO_DIAL.seat > 39.7 && STUDIO_BLOOM.seat[0] - STUDIO_BLOOM.seat[1] / 2 < 39.7 && STUDIO_FROST.ring[0] - STUDIO_FROST.ring[1] / 2 > 39.7);
  assert.ok(css.includes('html[data-look="studio"] .avatar > .ab-bloom,\nhtml[data-look="studio"] .avatar > .ab-frost,\nhtml[data-look="studio"] .avatar > .ab-dial { inset: -13%; }'));
  assert.ok(css.includes('html[data-look="studio"] .avatar > .ab-bloom::before,\nhtml[data-look="studio"] .avatar > .ab-frost::before,\nhtml[data-look="studio"] .avatar > .ab-dial::before { content: none; }'));
});

test("ornaments sit evenly on the ring, facing outwards", () => {
  const points = ornamentPoints(3, 180, 360);
  assert.deepEqual(points.map(p => [p.x, p.y]), [[0, 50], [50, 0], [100, 50]]);
  assert.deepEqual(points.map(p => p.angle), [270, 360, 450]);
  assert.deepEqual(ornamentPoints(1, 90, 90), [{ x: 50, y: 100, angle: 180 }]);
  for (const { count, from, to } of Object.values(STUDIO_ORNAMENTS)) {
    for (const p of ornamentPoints(count, from, to)) assert.ok(p.x >= 0 && p.x <= 100 && p.y >= 0 && p.y <= 100);
  }
});

test("a frame or an effect is the same drawing every time, and every part of it has a style", () => {
  for (const { id } of AVATAR_BORDERS) {
    assert.deepEqual(studioFrame(id), studioFrame(id));
    for (const part of studioFrame(id)) assert.ok(css.includes(`.${part.cls.split(" ")[0]} {`) || css.includes(`.${part.cls.split(" ")[0]},`), `${id} ${part.cls}`);
  }
  assert.deepEqual(Object.keys(STUDIO_FRAME_PARTS), ["orbit", "comet", "ripple"]);
  assert.deepEqual(studioFrame("ring"), []);
  assert.deepEqual(studioFrame("comet"), [{ cls: "ab-head" }]);
  assert.deepEqual(studioFrame("ripple"), [{ cls: "ab-wave" }, { cls: "ab-wave b" }]);
  for (const { id } of CARD_EFFECTS) {
    const parts = studioEffect(id);
    assert.deepEqual(parts, studioEffect(id));
    assert.ok(id === "aurora" ? parts.length === 0 : parts.length >= 7 && parts.length <= 31, id);
    for (const part of parts) {
      if (!part.vars) continue;
      assert.ok(part.vars.x >= -10 && part.vars.x <= 120, `${id} x`);
      assert.ok(part.vars.y >= -10 && part.vars.y <= 100, `${id} y`);
    }
  }
});
