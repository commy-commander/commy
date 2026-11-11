import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { INK_COLORS, INK_OWN, INK_TAG_INK, inkColor, inkStore, inkAdd, inkFrame, inkPaint, inkPen, inkStudio, inkTagBox } = require("../scripts/ink-view.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const page = read("../ink.html");
const script = read("../scripts/ink.js");
const app = read("../scripts/app.js");

const recorder = () => {
  const calls = [];
  const ctx = new Proxy({}, {
    get: (target, key) => key in target ? target[key] : (...args) => { calls.push([key, ...args, { ...target }]); return { width: 40 }; },
    set: (target, key, value) => { target[key] = value; return true; },
  });
  return { ctx, calls };
};

const paintOne = studio => {
  const store = inkStore();
  inkAdd(store, { from: "sam", name: "sam", mark: { kind: "line", stroke: 1, points: [[0, 0], [1, 1]] } }, 0, "host");
  inkAdd(store, { from: "lee", name: "nova", mark: { kind: "point", stroke: 0, points: [[0.5, 0.5]] } }, 0, "host");
  const { ctx, calls } = recorder();
  inkPaint(ctx, inkFrame(store, 0), { x: 0, y: 0, width: 200, height: 100 }, 1, studio);
  return { ctx, calls, pen: inkColor("lee") };
};

test("pens are the guide's Night text colours, with no red, and in the new look your own pen is ceramic", () => {
  assert.deepEqual(INK_COLORS, ["#f2a468", "#e9c96a", "#69d0c5", "#86d6a0", "#b7a9f7", "#f29bbd"]);
  assert.equal(INK_OWN, "#eef0f4");
  assert.equal(INK_COLORS.includes("#ff4d4d"), false);
  const store = inkStore();
  inkAdd(store, { from: "me", name: "", mark: { kind: "line", stroke: 1, points: [[0, 0], [1, 1]] } }, 0, "host");
  inkAdd(store, { from: "other", name: "o", mark: { kind: "point", stroke: 0, points: [[0.5, 0.5]] } }, 0, "host");
  const frame = inkFrame(store, 0, false, "me");
  assert.equal(frame.lines[0].own, true);
  assert.equal(frame.lines[0].color, inkColor("me"));
  assert.equal(frame.pings[0].own, false);
  assert.equal(inkPen(frame.lines[0], true), INK_OWN);
  assert.equal(inkPen(frame.lines[0], false), inkColor("me"));
  assert.equal(inkPen(frame.pings[0], true), inkColor("other"));
  assert.equal(inkFrame(store, 0).lines[0].own, false);
  const { ctx, calls } = recorder();
  inkPaint(ctx, frame, { x: 0, y: 0, width: 10, height: 10 }, 1, true);
  const stroke = calls.find(c => c[0] === "stroke");
  assert.equal(stroke[stroke.length - 1].strokeStyle, INK_OWN);
});

test("in the new look strokes and the dot carry a soft dark shadow and the ring fades as it grows", () => {
  const { calls } = paintOne(true);
  const stroke = calls.find(c => c[0] === "stroke");
  const state = stroke[stroke.length - 1];
  assert.equal(state.lineWidth, 4);
  assert.equal(state.lineCap, "round");
  assert.equal(state.shadowColor, "rgba(0, 0, 0, 0.65)");
  assert.equal(state.shadowOffsetY, 1);
  assert.equal(state.shadowBlur, 3);
  const dot = calls.find(c => c[0] === "arc" && c[3] === 7);
  assert.ok(dot);
  const ring = calls.filter(c => c[0] === "stroke")[1];
  assert.equal(ring[ring.length - 1].lineWidth, 3);
  assert.equal(ring[ring.length - 1].shadowColor, "transparent");
  assert.ok(Math.abs(ring[ring.length - 1].globalAlpha - 0.7) < 1e-9);
  const store = inkStore();
  inkAdd(store, { from: "lee", name: "", mark: { kind: "point", stroke: 0, points: [[0.5, 0.5]] } }, 0, "host");
  const later = recorder();
  inkPaint(later.ctx, inkFrame(store, 800), { x: 0, y: 0, width: 200, height: 100 }, 1, true);
  const grown = later.calls.filter(c => c[0] === "stroke")[0];
  assert.ok(grown[grown.length - 1].globalAlpha < 0.7);
  assert.equal(later.ctx.globalAlpha, 1);
  assert.equal(later.ctx.shadowColor, "transparent");
});

test("in the new look the name tag is a flag in the pen colour with dark Onest 700 12 ink", () => {
  const { calls, pen } = paintOne(true);
  const flag = calls.find(c => c[0] === "roundRect");
  assert.deepEqual(flag.slice(1, 6), [112, 62, 56, 22, [6, 6, 6, 2]]);
  assert.equal(flag[6].fillStyle, pen);
  assert.equal(flag[6].shadowColor, "rgba(0, 0, 0, 0.45)");
  const text = calls.find(c => c[0] === "fillText");
  assert.deepEqual(text.slice(1, 4), ["nova", 120, 73]);
  assert.equal(text[4].fillStyle, INK_TAG_INK);
  assert.equal(INK_TAG_INK, "#0b0d12");
  assert.equal(text[4].font, "700 12px Onest, system-ui, sans-serif");
  assert.deepEqual(inkTagBox(10, 10, 40, 2), { x: 34, y: 34, width: 40, height: 44, radii: [12, 12, 12, 4] });
});

test("the classic look keeps its tag and plain strokes", () => {
  const { calls } = paintOne(false);
  const stroke = calls.find(c => c[0] === "stroke");
  assert.equal(stroke[stroke.length - 1].shadowColor, undefined);
  assert.equal(calls.some(c => c[0] === "roundRect"), false);
  const box = calls.find(c => c[0] === "fillRect");
  assert.deepEqual(box.slice(1, 5), [112, 60, 54, 22]);
  assert.equal(box[5].fillStyle, "rgba(11, 13, 23, .82)");
  const text = calls.find(c => c[0] === "fillText");
  assert.equal(text[4].font, '600 13px Figtree, "Segoe UI", sans-serif');
  assert.equal(text[4].fillStyle, "#f2efe6");
  assert.equal(inkStudio(), false);
});

test("the drawing window knows the look and loads Onest", () => {
  const order = ["scripts/accessibility.js", "scripts/look.js", "scripts/ink-view.js", "scripts/ink.js"].map(name => page.indexOf(`<script src="${name}"></script>`));
  assert.ok(order.every(at => at > 0));
  assert.deepEqual([...order].sort((a, b) => a - b), order);
  assert.match(page, /@font-face \{ font-family: "Onest"; src: url\("fonts\/onest-latin-wght-normal\.woff2"\) format\("woff2"\); font-weight: 100 900; \}/);
  assert.match(script, /if \(e\.key === LOOK_KEY\) \{\n\s+applyLook\(\);\n\s+schedule\(\);/);
  assert.match(script, /inkFrame\(store, performance\.now\(\), calm, me\)/);
  assert.equal(script.includes("/".repeat(2)), false);
});

test("the in-app stream viewer and the voice room paint your own pen in ceramic", () => {
  assert.match(app, /inkFrame\(store, now, still, state\.user && state\.user\.id\)/);
  assert.match(app, /inkFrame\(ink\.store, performance\.now\(\), document\.documentElement\.hasAttribute\("data-reduced-motion"\), state\.user && state\.user\.id\)/);
});
