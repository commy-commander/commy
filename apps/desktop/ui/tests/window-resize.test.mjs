import "./i18n-global.mjs";
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const look = read("../scripts/look.js");
const phone = read("../scripts/studio-phone.js");

function countedDataset(writes) {
  return new Proxy({}, {
    set(target, name, value) {
      writes.push(`set ${String(name)}`);
      target[name] = value;
      return true;
    },
    deleteProperty(target, name) {
      writes.push(`delete ${String(name)}`);
      delete target[name];
      return true;
    },
  });
}

function fakeClassList(names, writes) {
  return {
    contains: name => names.has(name),
    add: name => {
      writes.push(`add ${name}`);
      names.add(name);
    },
    remove: name => {
      writes.push(`remove ${name}`);
      names.delete(name);
    },
  };
}

function fakeSheet(writes) {
  let media = "not all";
  return {
    get media() {
      return media;
    },
    set media(value) {
      writes.push(`media ${value}`);
      media = value;
    },
  };
}

function loadLook({ width = 1400, height = 900, phoneApp = false } = {}) {
  const writes = [];
  const listeners = {};
  const timers = new Map();
  let nextTimer = 1;
  const names = new Set(phoneApp ? ["mobile-app"] : []);
  const root = { dataset: countedDataset(writes), classList: fakeClassList(names, writes) };
  const sheets = [fakeSheet(writes), fakeSheet(writes)];
  const storage = { getItem: () => null, setItem: () => {} };
  const window = {
    innerWidth: width,
    innerHeight: height,
    localStorage: storage,
    sessionStorage: storage,
    addEventListener: (type, fn) => (listeners[type] = listeners[type] || []).push(fn),
  };
  const context = vm.createContext({
    window,
    document: { documentElement: root, querySelectorAll: () => sheets },
    MutationObserver: class {
      observe() {}
    },
    setTimeout: (fn, ms) => {
      const id = nextTimer++;
      timers.set(id, { fn, ms });
      return id;
    },
    clearTimeout: id => timers.delete(id),
  });
  vm.runInContext(look, context);
  const resize = () => (listeners.resize || []).forEach(fn => fn());
  return { context, window, writes, resize };
}

test("resizing within the same fit writes nothing to the page root, so its watchers stay quiet", () => {
  const app = loadLook();
  assert.ok(app.writes.includes("set fit"));
  app.writes.length = 0;
  for (const width of [1390, 1350, 1300, 1250, 1200]) {
    app.window.innerWidth = width;
    app.context.applyLook();
  }
  assert.deepEqual(app.writes, []);
});

test("a resize event goes straight to the look with nothing added to the page", () => {
  const app = loadLook();
  app.writes.length = 0;
  app.window.innerWidth = 1300;
  app.resize();
  assert.deepEqual(app.writes, []);
});

test("crossing into a new fit still changes the fit once", () => {
  const app = loadLook();
  app.writes.length = 0;
  app.window.innerWidth = 1000;
  app.context.applyLook();
  app.context.applyLook();
  assert.deepEqual(app.writes, ["set fit"]);
});

test("the chat pane only writes its measurements when they change", () => {
  const start = phone.indexOf("function setStyleVar");
  const source = phone.slice(start, phone.indexOf("\n}\n", start) + 3);
  const context = vm.createContext({});
  vm.runInContext(source, context);
  const writes = [];
  const values = {};
  const node = {
    style: {
      getPropertyValue: name => values[name] || "",
      setProperty: (name, value) => {
        writes.push(name);
        values[name] = value;
      },
    },
  };
  context.setStyleVar(node, "--pt-pane-y", "66px");
  context.setStyleVar(node, "--pt-pane-y", "66px");
  context.setStyleVar(node, "--pt-pane-y", "70px");
  assert.deepEqual(writes, ["--pt-pane-y", "--pt-pane-y"]);
});

function loadRoom({ width = 1400, height = 900 } = {}) {
  const { holdStudioRoom, STUDIO_ROOM_SETTLE_MS } = require("../scripts/studio-room.js");
  const listeners = {};
  const timers = new Map();
  let nextTimer = 1;
  const style = {};
  const art = { style };
  const win = {
    innerWidth: width,
    innerHeight: height,
    addEventListener: (type, fn) => (listeners[type] = listeners[type] || []).push(fn),
    setTimeout: (fn, ms) => {
      const id = nextTimer++;
      timers.set(id, { fn, ms });
      return id;
    },
    clearTimeout: id => timers.delete(id),
  };
  const room = { querySelector: selector => (selector === "svg" ? art : null) };
  holdStudioRoom(room, win);
  const resizeTo = (w, h) => {
    win.innerWidth = w;
    win.innerHeight = h;
    (listeners.resize || []).forEach(fn => fn());
  };
  const settle = () => {
    const pending = [...timers.values()];
    timers.clear();
    for (const timer of pending) {
      assert.equal(timer.ms, STUDIO_ROOM_SETTLE_MS);
      timer.fn();
    }
    return pending.length;
  };
  return { style, resizeTo, settle };
}

const RESTING = { position: "absolute", left: "50%", top: "50%", willChange: "transform", width: "100%", height: "100%", transform: "translate(-50%, -50%)" };

test("the held room art rests centred on its own layer at the window's size", () => {
  const room = loadRoom();
  assert.deepEqual(room.style, RESTING);
});

test("while the window is dragged the room art keeps the size it was drawn at and is only scaled to cover", () => {
  const room = loadRoom();
  room.resizeTo(1200, 800);
  room.resizeTo(1000, 760);
  assert.deepEqual(room.style, { ...RESTING, width: "1400px", height: "900px", transform: `translate(-50%, -50%) scale(${760 / 900})` });
});

test("the room art is drawn again at the new size once the drag settles, and only once", () => {
  const room = loadRoom();
  for (const width of [1380, 1340, 1300, 1260]) room.resizeTo(width, 900);
  assert.equal(room.settle(), 1);
  assert.deepEqual(room.style, RESTING);
  room.resizeTo(1200, 800);
  assert.deepEqual(room.style, { ...RESTING, width: "1260px", height: "900px", transform: `translate(-50%, -50%) scale(${1200 / 1260})` });
});

test("the room art is only held in the Chromium webview, so WebKit keeps drawing it as before", () => {
  const { roomHoldHelps } = require("../scripts/studio-room.js");
  assert.equal(roomHoldHelps({ userAgentData: { brands: [] } }), true);
  assert.equal(roomHoldHelps({ userAgent: "AppleWebKit/605.1.15" }), false);
  assert.equal(roomHoldHelps(undefined), false);
  assert.equal(read("../styles/studio.css").includes("will-change"), false);
});
