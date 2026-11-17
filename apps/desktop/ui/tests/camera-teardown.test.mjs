import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../scripts/camera-pipeline.js", import.meta.url), "utf8");
const start = source.indexOf("function detachCameraFeed");
const end = source.indexOf("function drawCameraFrame");

function loadTeardown(log) {
  const context = vm.createContext({ clearInterval: () => log.push("timer cleared") });
  vm.runInContext(source.slice(start, end), context);
  return context;
}

function fakeStream(name, log) {
  return { getTracks: () => [{ stop: () => log.push(`${name} track stopped`) }] };
}

function fakePipe(log) {
  let srcObject = null;
  const video = {
    pause: () => log.push("video paused"),
    play: () => {
      log.push("video played");
      return Promise.resolve();
    },
    remove: () => log.push("video removed"),
    get srcObject() {
      return srcObject;
    },
    set srcObject(value) {
      srcObject = value;
      log.push(value ? `video shows ${value.name}` : "video detached");
    },
  };
  const stream = { ...fakeStream("old", log), name: "old" };
  video.srcObject = stream;
  log.length = 0;
  return { stream, video, canvas: { remove: () => log.push("canvas removed") }, timer: 1 };
}

test("stopping the camera detaches the feed video before stopping its tracks", () => {
  const log = [];
  const { stopCameraPipeline } = loadTeardown(log);
  const pipe = fakePipe(log);
  stopCameraPipeline(pipe);
  assert.deepEqual(log, ["timer cleared", "video paused", "video detached", "old track stopped", "video removed", "canvas removed"]);
  assert.equal(pipe.timer, null);
});

test("switching cameras detaches the feed video before stopping the old tracks", () => {
  const log = [];
  const { swapCameraSource } = loadTeardown(log);
  const pipe = fakePipe(log);
  const next = { ...fakeStream("new", log), name: "new" };
  swapCameraSource(pipe, next);
  assert.deepEqual(log, ["video paused", "video detached", "old track stopped", "video shows new", "video played"]);
  assert.equal(pipe.stream, next);
});
