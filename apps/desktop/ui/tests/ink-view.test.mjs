import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";

const require = createRequire(import.meta.url);
const {
  INK_MAX_POINTS, INK_LINE_HOLD_MS, INK_FADE_MS, INK_POINT_MS, INK_COLORS,
  inkColor, inkStore, inkAdd, inkFrame, inkFit, inkToFrame, inkFarEnough, inkChunks, inkPaint, inkSettingOn, inkScreenOn,
} = require("../scripts/ink-view.js");

const root = new URL("../../", import.meta.url);
const read = path => readFileSync(new URL(path, root), "utf8");

const line = (from, stroke, points) => ({ from, name: from, mark: { kind: "line", stroke, points } });
const point = (from, x, y) => ({ from, name: from, mark: { kind: "point", stroke: 0, points: [[x, y]] } });
const clear = from => ({ from, mark: { kind: "clear", stroke: 0, points: [] } });

test("the limits match the app and the window it draws in", () => {
  const media = read("../../crates/commy-net/src/media.rs");
  assert.match(media, new RegExp(`pub const MAX_INK_POINTS: usize = ${INK_MAX_POINTS};`));
  assert.match(read("src-tauri/src/ink_window.rs"), /pub const LABEL: &str = "stream-ink";/);
  assert.ok(JSON.parse(read("src-tauri/capabilities/default.json")).windows.includes("stream-ink"));
  assert.match(read("src-tauri/src/ink_window.rs"), /WebviewUrl::App\("ink\.html"\.into\(\)\)/);
  assert.match(read("src-tauri/src/ink_window.rs"), /\.content_protected\(true\)/);
  assert.match(read("../web/build.mjs"), /"apps\/desktop\/ui\/ink\.html"/);
});

test("strokes grow across packets and fade after a pause", () => {
  const store = inkStore();
  inkAdd(store, line("sam", 1, [[0.1, 0.1]]), 0, "host");
  inkAdd(store, line("sam", 1, [[0.2, 0.2], [0.3, 0.3]]), 100, "host");
  inkAdd(store, line("sam", 2, [[0.9, 0.9]]), 100, "host");
  let frame = inkFrame(store, 200);
  assert.equal(frame.lines.length, 2);
  assert.deepEqual(frame.lines[0].points, [[0.1, 0.1], [0.2, 0.2], [0.3, 0.3]]);
  assert.equal(frame.lines[0].alpha, 1);
  frame = inkFrame(store, 100 + INK_LINE_HOLD_MS + INK_FADE_MS / 2);
  assert.ok(frame.lines[0].alpha > 0 && frame.lines[0].alpha < 1);
  frame = inkFrame(store, 100 + INK_LINE_HOLD_MS + INK_FADE_MS + 1);
  assert.equal(frame.lines.length, 0);
  assert.equal(store.lines.size, 0);
  assert.equal(frame.busy, false);
});

test("a point keeps one ping per person, with their name, and goes away on its own", () => {
  const store = inkStore();
  inkAdd(store, point("sam", 0.5, 0.5), 0, "host");
  inkAdd(store, point("sam", 0.6, 0.4), 10, "host");
  inkAdd(store, point("lee", 2, -1), 10, "host");
  const frame = inkFrame(store, 20);
  assert.equal(frame.pings.length, 2);
  assert.deepEqual([frame.pings[0].x, frame.pings[0].y, frame.pings[0].name], [0.6, 0.4, "sam"]);
  assert.deepEqual([frame.pings[1].x, frame.pings[1].y], [1, 0]);
  assert.equal(inkFrame(store, 10 + INK_POINT_MS).pings.length, 0);
});

test("clearing from a viewer wipes only their marks, and from the streamer wipes everything", () => {
  const store = inkStore();
  inkAdd(store, line("sam", 1, [[0.1, 0.1]]), 0, "host");
  inkAdd(store, line("lee", 1, [[0.2, 0.2]]), 0, "host");
  inkAdd(store, point("sam", 0.5, 0.5), 0, "host");
  inkAdd(store, clear("sam"), 1, "host");
  assert.deepEqual([...store.lines.values()].map(l => l.from), ["lee"]);
  assert.equal(store.pings.length, 0);
  inkAdd(store, clear("host"), 2, "host");
  assert.equal(store.lines.size, 0);
});

test("reduced motion drops the fade and the growing ring", () => {
  const store = inkStore();
  inkAdd(store, line("sam", 1, [[0.1, 0.1]]), 0, "host");
  inkAdd(store, point("sam", 0.5, 0.5), 0, "host");
  const frame = inkFrame(store, INK_POINT_MS / 2, true);
  assert.equal(frame.pings[0].progress, 0);
  assert.equal(inkFrame(store, INK_LINE_HOLD_MS + 2, true).lines.length, 0);
});

test("the video's shown area is found inside its box and pointers map to it", () => {
  const rect = inkFit(1920, 1080, 960, 1000);
  assert.deepEqual(rect, { x: 0, y: 230, width: 960, height: 540 });
  assert.deepEqual(inkToFrame(480, 500, rect), [0.5, 0.5]);
  assert.equal(inkToFrame(500, 100, rect), null);
  assert.deepEqual(inkFit(0, 0, 300, 200), { x: 0, y: 0, width: 300, height: 200 });
  assert.equal(inkToFrame(1, 1, { x: 0, y: 0, width: 0, height: 0 }), null);
});

test("pen points are thinned and sent in pieces the app accepts", () => {
  assert.equal(inkFarEnough(null, [0, 0]), true);
  assert.equal(inkFarEnough([0, 0], [0.0005, 0]), false);
  assert.equal(inkFarEnough([0, 0], [0.01, 0]), true);
  const points = Array.from({ length: INK_MAX_POINTS * 2 + 3 }, (_, i) => [i / 1000, 0]);
  const chunks = inkChunks(points);
  assert.deepEqual(chunks.map(c => c.length), [INK_MAX_POINTS, INK_MAX_POINTS, 3]);
  assert.deepEqual(chunks.flat(), points);
});

test("each person keeps one colour", () => {
  assert.equal(inkColor("abc"), inkColor("abc"));
  assert.ok(INK_COLORS.includes(inkColor("someone")));
  const spread = new Set(Array.from({ length: 40 }, (_, i) => inkColor(`user-${i}`)));
  assert.ok(spread.size > 3);
});

test("painting draws lines and pings onto the shown area", () => {
  const calls = [];
  const ctx = new Proxy({}, {
    get: (target, key) => key in target ? target[key] : (...args) => { calls.push([key, ...args]); return { width: 40 }; },
    set: (target, key, value) => { target[key] = value; return true; },
  });
  const store = inkStore();
  inkAdd(store, line("sam", 1, [[0, 0], [1, 1]]), 0, "host");
  inkAdd(store, point("lee", 0.5, 0.5), 0, "host");
  inkPaint(ctx, inkFrame(store, 0), { x: 10, y: 20, width: 100, height: 50 });
  assert.deepEqual(calls.find(c => c[0] === "moveTo"), ["moveTo", 10, 20]);
  assert.deepEqual(calls.find(c => c[0] === "lineTo"), ["lineTo", 110, 70]);
  assert.ok(calls.some(c => c[0] === "arc" && c[1] === 60 && c[2] === 45));
  assert.ok(calls.some(c => c[0] === "fillText" && c[1] === "lee"));
  assert.equal(ctx.globalAlpha, 1);
});

test("drawing is off until turned on, and drawings show on screen unless turned off", () => {
  assert.equal(inkSettingOn(null), false);
  assert.equal(inkSettingOn("1"), true);
  assert.equal(inkScreenOn(null), true);
  assert.equal(inkScreenOn("0"), false);
});

test("the stage, share picker, voice panel and settings carry the drawing controls", () => {
  const html = read("ui/index.html");
  for (const id of ["stage-ink", "stage-ink-tools", "stage-ink-point", "stage-ink-pen", "stage-ink-clear", "stage-drawers", "share-ink-row", "share-ink", "vp-ink-row", "vp-ink", "stream-ink-toggle", "stream-ink-screen-toggle"]) {
    assert.ok(html.includes(`id="${id}"`), `page lacks #${id}`);
  }
  assert.ok(html.indexOf('src="scripts/ink-view.js"') < html.indexOf('src="scripts/app.js"'));
  const page = read("ui/ink.html");
  assert.ok(page.indexOf("scripts/ink-view.js") < page.indexOf("scripts/ink.js"));
});
