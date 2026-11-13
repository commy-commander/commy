import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  STARTUP_STEPS, STARTUP_MIN_SHOWN_MS, STARTUP_STEP_GAP_MS,
  readStartup, readStartupStep, startupTitle, startupPreview, emptyStartupPace, pacedStartup,
} = require("../scripts/startup.js");

const ui = join(import.meta.dirname, "..");
const page = readFileSync(join(ui, "index.html"), "utf8");
const backend = readFileSync(join(ui, "..", "src-tauri", "src", "startup.rs"), "utf8");
const app = readFileSync(join(ui, "scripts", "app.js"), "utf8");

test("no view, as in the browser, opens the app at once", () => {
  assert.equal(readStartup(null), null);
  assert.equal(readStartup("nope"), null);
});

test("a view from the backend is read step by step", () => {
  const view = readStartup({
    connection: { state: "done", detail: "You are online", progress: null },
    update: { state: "working", detail: "Downloading Commy 0.2.1", progress: 42.4 },
    sync: { state: "working", detail: "Catching up 2 servers" },
    offline: false, restarting: false, ready: false, can_skip: true,
  });
  assert.equal(view.connection.state, "done");
  assert.equal(view.update.progress, 42);
  assert.equal(view.sync.progress, null);
  assert.equal(view.canSkip, true);
  assert.equal(view.ready, false);
});

test("odd steps fall back to waiting with nothing said", () => {
  assert.deepEqual(readStartupStep({ state: "exploded", detail: 7, progress: 900 }), { state: "waiting", detail: "", progress: 100 });
  assert.deepEqual(readStartupStep(undefined), { state: "waiting", detail: "", progress: null });
});

test("the title says what is happening", () => {
  assert.equal(startupTitle(null), "Starting Commy");
  assert.equal(startupTitle(readStartup({ restarting: true })), "Restarting to finish the update");
  assert.equal(startupTitle(readStartup({ ready: true, offline: true })), "Opening Commy offline");
  assert.equal(startupTitle(readStartup({ ready: true })), "Opening Commy");
});

test("the page has a row for every step, and the backend sends every step", () => {
  for (const key of STARTUP_STEPS) {
    assert.match(page, new RegExp(`id="startup-${key}"`));
    assert.match(backend, new RegExp(`pub ${key}: Step`));
  }
});

test("the backend's step states are ones the page can draw", () => {
  const block = /pub enum StepState \{([\s\S]*?)\}/.exec(backend)[1];
  const states = [...block.matchAll(/(\w+),/g)].map(m => m[1].toLowerCase());
  for (const state of states) assert.match(readFileSync(join(ui, "styles", "styles.css"), "utf8"), new RegExp(`data-state="${state}"`));
});

test("the loader covers the page before any script runs and offers a way past it", () => {
  const body = page.indexOf("<body>");
  assert.ok(page.indexOf('id="startup"') > body);
  assert.ok(page.indexOf('id="startup"') < page.indexOf('id="app"'));
  assert.match(page, /id="startup-skip"/);
  assert.ok(page.indexOf("scripts/startup.js") < page.indexOf("scripts/app.js"));
});

test("the preview opens once everything is done", () => {
  assert.equal(readStartup(startupPreview(null, 0)).ready, false);
  const done = readStartup(startupPreview(null, 5000));
  assert.equal(done.ready, true);
  for (const key of STARTUP_STEPS) assert.equal(done[key].state, "done");
});

test("the offline preview still opens, and says it is offline", () => {
  const view = readStartup(startupPreview("offline", 2000));
  assert.equal(view.ready, true);
  assert.equal(view.offline, true);
  assert.equal(view.connection.state, "failed");
  assert.equal(startupTitle(view), "Opening Commy offline");
});

test("the update preview downloads with progress, then restarts instead of opening", () => {
  const downloading = readStartup(startupPreview("update", 2500));
  assert.equal(downloading.update.progress, 50);
  const restarting = readStartup(startupPreview("update", 6000));
  assert.equal(restarting.restarting, true);
  assert.equal(restarting.ready, false);
  assert.equal(restarting.canSkip, false);
});

const TICK_MS = 50;

function backendView(steps, extra = {}) {
  const raw = { offline: false, restarting: false, can_skip: false, ...extra };
  for (const key of STARTUP_STEPS) raw[key] = { state: steps[key][0], detail: steps[key][1] };
  raw.ready = STARTUP_STEPS.every(key => !["working", "waiting"].includes(steps[key][0]));
  return readStartup(raw);
}

const allDone = backendView({
  connection: ["done", "You are online"],
  update: ["done", "You have the latest version"],
  sync: ["done", "Everything is up to date"],
});

function playStartup(backendAt, until = 10000) {
  let pace = emptyStartupPace();
  const shownSettled = {};
  for (let now = 0; now <= until; now += TICK_MS) {
    const paced = pacedStartup(backendAt(now), pace, 0, now);
    pace = paced.pace;
    for (const key of STARTUP_STEPS) {
      if (shownSettled[key] == null && paced.view[key].state !== "working") shownSettled[key] = now;
    }
    if (paced.canClose) return { shownSettled, closedAt: now, last: paced.view };
  }
  return { shownSettled, closedAt: null };
}

test("the pacing waits half a second per step and two seconds before closing", () => {
  assert.equal(STARTUP_STEP_GAP_MS, 500);
  assert.equal(STARTUP_MIN_SHOWN_MS, 2000);
});

test("a backend that is ready at once still ticks each step half a second apart and closes at two seconds", () => {
  const { shownSettled, closedAt, last } = playStartup(() => allDone);
  assert.deepEqual(shownSettled, { connection: 500, update: 1000, sync: 1500 });
  assert.equal(closedAt, 2000);
  assert.equal(last.ready, true);
  assert.equal(startupTitle(last), "Opening Commy");
});

test("nothing closes and no step shows done before its turn", () => {
  const early = pacedStartup(allDone, emptyStartupPace(), 0, 0);
  assert.equal(early.canClose, false);
  assert.equal(early.view.ready, false);
  assert.equal(startupTitle(early.view), "Starting Commy");
  for (const key of STARTUP_STEPS) assert.equal(early.view[key].state, "working");
});

test("a step held back keeps saying what it was last doing", () => {
  const checking = backendView({
    connection: ["working", "Checking your connection"],
    update: ["working", "Looking for a new version"],
    sync: ["working", "Connecting to the sync service"],
  });
  const first = pacedStartup(checking, emptyStartupPace(), 0, 0);
  const held = pacedStartup(allDone, first.pace, 0, 100);
  assert.deepEqual(held.view.update, { state: "working", detail: "Looking for a new version", progress: null });
  assert.deepEqual(held.view.sync, { state: "working", detail: "Connecting to the sync service", progress: null });
});

test("a step never seen working is held with its own words", () => {
  const held = pacedStartup(allDone, emptyStartupPace(), 0, 100);
  assert.deepEqual(held.view.update, { state: "working", detail: "You have the latest version", progress: null });
});

test("a late step pushes the ones after it back so they never land together", () => {
  const lateConnection = now => backendView({
    connection: now < 1200 ? ["working", "Checking your connection"] : ["done", "You are online"],
    update: ["skipped", "Development builds do not update themselves"],
    sync: ["done", "Everything is up to date"],
  });
  const { shownSettled, closedAt } = playStartup(lateConnection);
  assert.deepEqual(shownSettled, { connection: 1200, update: 1700, sync: 2200 });
  assert.equal(closedAt, 2200);
});

test("a slow backend closes as soon as it is ready", () => {
  const slowSync = now => backendView({
    connection: ["done", "You are online"],
    update: ["done", "You have the latest version"],
    sync: now < 5000 ? ["working", "Loading 1 server"] : ["done", "Everything is up to date"],
  });
  const { shownSettled, closedAt } = playStartup(slowSync);
  assert.deepEqual(shownSettled, { connection: 500, update: 1000, sync: 5000 });
  assert.equal(closedAt, 5000);
});

test("restarting into an update is shown as it is, and never closes", () => {
  const restarting = backendView({
    connection: ["done", "You are online"],
    update: ["working", "Restarting into Commy 0.2.4"],
    sync: ["done", "Everything is up to date"],
  }, { restarting: true });
  const paced = pacedStartup(restarting, emptyStartupPace(), 0, 0);
  assert.equal(paced.view, restarting);
  assert.equal(paced.canClose, false);
});

test("the default preview scene also lands its steps one by one", () => {
  const { shownSettled, closedAt } = playStartup(now => readStartup(startupPreview(null, now)));
  assert.ok(shownSettled.update - shownSettled.connection >= STARTUP_STEP_GAP_MS);
  assert.ok(shownSettled.sync - shownSettled.update >= STARTUP_STEP_GAP_MS);
  assert.ok(closedAt >= STARTUP_MIN_SHOWN_MS);
});

test("the loader closes from the paced view, keeps skip and the offline linger, and opens at once without a backend", () => {
  const loader = /function runStartupLoader\(\) \{[\s\S]*?\r?\n\}\r?\n/.exec(app)[0];
  assert.match(loader, /pacedStartup\(last, pace, shownAt, Date\.now\(\)\)/);
  assert.match(loader, /if \(!paced\.canClose\) return;/);
  assert.match(loader, /last\.offline \? STARTUP_OFFLINE_LINGER_MS : 0/);
  assert.match(loader, /if \(!view\) return finish\(\);/);
  assert.match(loader, /\$\("startup-skip"\)\.addEventListener\("click", finish\)/);
  assert.doesNotMatch(loader, /if \(view\.ready\)/);
});
