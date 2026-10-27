import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import vm from "node:vm";

const source = readFileSync(join(import.meta.dirname, "..", "scripts", "plex-sync.js"), "utf8");

class FakeVideo {
  constructor(width, height) {
    this.clientWidth = width;
    this.clientHeight = height;
    this.paused = true;
    this.currentTime = 0;
    this.listeners = new Map();
  }
  addEventListener(name, fn) {
    this.listeners.set(name, fn);
  }
  removeEventListener(name) {
    this.listeners.delete(name);
  }
  fire(name) {
    const fn = this.listeners.get(name);
    if (fn) fn();
  }
  play() {
    this.paused = false;
    this.fire("play");
    return Promise.resolve();
  }
  pause() {
    this.paused = true;
    this.fire("pause");
  }
}

function page(host, videos, clock) {
  const window = {};
  const context = {
    window,
    location: { hostname: host },
    document: { querySelectorAll: () => videos },
    setInterval: () => 0,
    Date: { now: () => clock.now },
    Object,
    Number,
    Boolean,
  };
  vm.runInNewContext(source, context);
  return window.__commyWatch;
}

test("the sync script stays out of every site but Plex's web app", () => {
  assert.equal(page("www.youtube.com", [], { now: 0 }), undefined);
  assert.equal(page("plex.tv", [], { now: 0 }), undefined);
  assert.ok(page("app.plex.tv", [], { now: 0 }));
});

test("presses on the biggest video are queued once and handed over", () => {
  const clock = { now: 1000 };
  const trailer = new FakeVideo(0, 0);
  const film = new FakeVideo(1280, 720);
  const watch = page("app.plex.tv", [trailer, film], clock);
  assert.deepEqual(JSON.parse(JSON.stringify(watch.take())), { events: [], video: true, playing: false });
  film.currentTime = 42;
  film.play();
  film.pause();
  trailer.play();
  const taken = JSON.parse(JSON.stringify(watch.take()));
  assert.deepEqual(taken.events, [{ playing: true, position: 42 }, { playing: false, position: 42 }]);
  assert.equal(taken.playing, false);
  assert.deepEqual(JSON.parse(JSON.stringify(watch.take())).events, []);
});

test("play and pause sent from Commy are not echoed back as presses", () => {
  const clock = { now: 1000 };
  const film = new FakeVideo(1280, 720);
  const watch = page("app.plex.tv", [film], clock);
  watch.apply(true);
  assert.equal(film.paused, false);
  assert.deepEqual(JSON.parse(JSON.stringify(watch.take())).events, []);
  watch.apply(true);
  clock.now += 5000;
  film.pause();
  assert.deepEqual(JSON.parse(JSON.stringify(watch.take())).events, [{ playing: false, position: 0 }]);
});

test("with no video yet, Commy's play and pause wait for the title to start", () => {
  const watch = page("app.plex.tv", [], { now: 0 });
  watch.apply(true);
  assert.deepEqual(JSON.parse(JSON.stringify(watch.take())), { events: [], video: false, playing: false });
});

test("the page cannot swap out the hand-off", () => {
  const watch = page("app.plex.tv", [], { now: 0 });
  assert.ok(Object.isFrozen(watch));
});
