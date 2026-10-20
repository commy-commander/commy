import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  CAMERA_BACKGROUNDS,
  parseCameraFx,
  cameraFxNeedsSegmenter,
  edgeStep,
  writeMaskAlpha,
  maskMemory,
  segmentStride,
  averageCost,
  cameraFrameSize,
} = require("../scripts/camera-fx.js");

const ui = join(import.meta.dirname, "..");

test("camera settings fall back to no effect and a mirrored self view", () => {
  assert.deepEqual(parseCameraFx(null), { background: "off", mirror: true });
  assert.deepEqual(parseCameraFx("blur"), { background: "off", mirror: true });
  assert.deepEqual(parseCameraFx({ background: "confetti", mirror: "yes" }), { background: "off", mirror: true });
  assert.deepEqual(parseCameraFx({ background: "blur", mirror: false }), { background: "blur", mirror: false });
  assert.deepEqual(parseCameraFx({ background: "stars" }), { background: "off", mirror: true });
  assert.deepEqual(CAMERA_BACKGROUNDS, ["off", "blur"]);
});

test("only blur loads the person segmenter", () => {
  assert.equal(cameraFxNeedsSegmenter({ background: "off" }), false);
  assert.equal(cameraFxNeedsSegmenter({ background: "blur" }), true);
  assert.equal(cameraFxNeedsSegmenter(undefined), false);
});

test("mask edges are soft between unsure and sure", () => {
  assert.equal(edgeStep(0), 0);
  assert.equal(edgeStep(0.3), 0);
  assert.ok(Math.abs(edgeStep(0.5) - 0.5) < 1e-9);
  assert.equal(edgeStep(0.7), 1);
  assert.equal(edgeStep(1), 1);
  assert.ok(edgeStep(0.4) > 0 && edgeStep(0.4) < 0.5);
});

test("the first mask is taken as is, later masks ease toward the new shape", () => {
  const memory = maskMemory(2);
  const rgba = new Uint8ClampedArray(8);
  writeMaskAlpha(new Float32Array([1, 0]), memory, rgba);
  assert.deepEqual([...rgba], [255, 255, 255, 255, 255, 255, 255, 0]);
  writeMaskAlpha(new Float32Array([0, 1]), memory, rgba, 0.4);
  assert.equal(rgba[3], Math.round(0.4 * 255));
  assert.equal(rgba[7], Math.round(0.6 * 255));
});

test("slow segmentation is spread over more frames, up to every fourth", () => {
  assert.equal(segmentStride(0), 1);
  assert.equal(segmentStride(NaN), 1);
  assert.equal(segmentStride(4), 1);
  assert.equal(segmentStride(20), 1);
  assert.equal(segmentStride(25), 2);
  assert.equal(segmentStride(500), 4);
  assert.equal(averageCost(0, 30), 30);
  assert.equal(averageCost(10, 20), 12);
});

test("frames are capped at the send width and keep their shape", () => {
  assert.equal(cameraFrameSize(0, 360, 640), null);
  assert.deepEqual(cameraFrameSize(640, 360, 640), { width: 640, height: 360 });
  assert.deepEqual(cameraFrameSize(1280, 720, 640), { width: 640, height: 360 });
  assert.deepEqual(cameraFrameSize(320, 240, 640), { width: 320, height: 240 });
});

test("every file the background effects load ships with the app", () => {
  const source = readFileSync(join(ui, "scripts", "camera-pipeline.js"), "utf8");
  const block = /const CAMERA_FX_FILES = \{([\s\S]*?)\};/.exec(source)[1];
  const paths = [...block.matchAll(/"(vendor\/[^"]+)"/g)].map(m => m[1]);
  assert.equal(paths.length, 4);
  for (const path of paths) assert.ok(existsSync(join(ui, path)), `${path} is missing`);
  assert.ok(existsSync(join(ui, "vendor", "mediapipe", "LICENSE")));
});

test("the bundled segmenter never phones home", () => {
  const bundle = readFileSync(join(ui, "vendor", "mediapipe", "vision_bundle.mjs"), "utf8");
  const loader = readFileSync(join(ui, "vendor", "mediapipe", "wasm", "vision_wasm_internal.js"), "utf8");
  assert.doesNotMatch(bundle, /https?:\/\//);
  for (const text of [bundle, loader]) {
    assert.doesNotMatch(text, /odml|sendBeacon|\.pa\.googleapis/);
  }
});

test("the camera scripts load before the app that uses them", () => {
  const html = readFileSync(join(ui, "index.html"), "utf8");
  const order = ["scripts/camera-fx.js", "scripts/camera-pipeline.js", "scripts/app.js"].map(src => html.indexOf(`<script src="${src}">`));
  assert.ok(order.every(i => i > 0));
  assert.deepEqual([...order].sort((a, b) => a - b), order);
});
