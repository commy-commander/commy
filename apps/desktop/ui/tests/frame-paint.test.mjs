import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { base64Bytes, createLatestOnly, createFramePainter } = require("../scripts/frame-paint.js");

function deferred() {
  let resolve;
  const promise = new Promise(r => (resolve = r));
  return { promise, resolve };
}

const tick = () => new Promise(r => setTimeout(r, 0));

test("frames that arrive while one is decoding are dropped except the newest", async () => {
  const started = [];
  const gates = [];
  const queue = createLatestOnly(item => {
    started.push(item);
    const gate = deferred();
    gates.push(gate);
    return gate.promise;
  });
  queue.push(1);
  queue.push(2);
  queue.push(3);
  queue.push(4);
  assert.deepEqual(started, [1]);
  assert.equal(queue.busy, true);
  gates[0].resolve();
  await tick();
  assert.deepEqual(started, [1, 4]);
  assert.equal(queue.dropped, 2);
  gates[1].resolve();
  await tick();
  assert.equal(queue.busy, false);
  queue.push(5);
  assert.deepEqual(started, [1, 4, 5]);
});

test("a failed decode does not stop later frames", async () => {
  const seen = [];
  const queue = createLatestOnly(async item => {
    seen.push(item);
    if (item === "bad") throw new Error("broken jpeg");
  });
  queue.push("bad");
  await tick();
  queue.push("good");
  await tick();
  assert.deepEqual(seen, ["bad", "good"]);
  assert.equal(queue.busy, false);
});

function fakeCanvas() {
  const drawn = [];
  const classes = new Set(["blank"]);
  return {
    width: 300,
    height: 150,
    drawn,
    classList: { add: c => classes.add(c), remove: c => classes.delete(c), has: c => classes.has(c) },
    getContext: () => ({ drawImage: bitmap => drawn.push(bitmap.id) }),
  };
}

test("every decoded frame is closed after it is drawn, so its memory is freed at once", async () => {
  const canvas = fakeCanvas();
  const bitmaps = [];
  const painted = [];
  const painter = createFramePainter(canvas, {
    decode: async bytes => {
      const bitmap = { id: bytes[0], width: 1600, height: 900, closed: false, close() { this.closed = true; } };
      bitmaps.push(bitmap);
      return bitmap;
    },
    onPaint: (w, h) => painted.push(`${w}x${h}`),
  });
  painter.paint(btoa("\x07"));
  await tick();
  painter.paint(btoa("\x08"));
  await tick();
  assert.deepEqual(canvas.drawn, [7, 8]);
  assert.equal(canvas.width, 1600);
  assert.equal(canvas.height, 900);
  assert.equal(canvas.classList.has("blank"), false);
  assert.deepEqual(painted, ["1600x900", "1600x900"]);
  assert.ok(bitmaps.every(b => b.closed));
});

test("a frame still decoding when the stage closes is never drawn", async () => {
  const canvas = fakeCanvas();
  const gate = deferred();
  const bitmap = { id: 1, width: 10, height: 10, closed: false, close() { this.closed = true; } };
  const painter = createFramePainter(canvas, { decode: () => gate.promise.then(() => bitmap) });
  painter.paint(btoa("\x01"));
  await tick();
  painter.clear();
  gate.resolve();
  await tick();
  assert.deepEqual(canvas.drawn, []);
  assert.equal(canvas.width, 0);
  assert.equal(canvas.classList.has("blank"), true);
  assert.equal(bitmap.closed, true);
});

test("frame bytes come through unchanged", () => {
  assert.deepEqual([...base64Bytes(btoa("\xff\xd8\x00\x10"))], [0xff, 0xd8, 0x00, 0x10]);
});

test("a frame can be fetched only when the painter is ready for it, and an empty answer draws nothing", async () => {
  const canvas = fakeCanvas();
  const fetched = [];
  const painter = createFramePainter(canvas, {
    decode: async bytes => ({ id: bytes[0], width: 4, height: 3, close() {} }),
  });
  painter.paint(async () => { fetched.push("a"); return new Uint8Array([5]).buffer; });
  painter.paint(async () => { fetched.push("b"); return new Uint8Array([6]).buffer; });
  painter.paint(async () => { fetched.push("c"); return new Uint8Array([7]).buffer; });
  await tick();
  await tick();
  assert.deepEqual(fetched, ["a", "c"]);
  assert.deepEqual(canvas.drawn, [5, 7]);
  painter.paint(async () => new ArrayBuffer(0));
  await tick();
  assert.deepEqual(canvas.drawn, [5, 7]);
});

test("frame data can be base64, bytes or a fetch", async () => {
  const { frameBytes } = require("../scripts/frame-paint.js");
  assert.deepEqual([...await frameBytes(btoa("\x01\x02"))], [1, 2]);
  assert.deepEqual([...await frameBytes(new Uint8Array([3]))], [3]);
  assert.deepEqual([...await frameBytes(async () => new Uint8Array([4]).buffer)], [4]);
});
