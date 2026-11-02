import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { STARTUP_STEPS, readStartup, readStartupStep, startupTitle, startupPreview } = require("../scripts/startup.js");

const ui = join(import.meta.dirname, "..");
const page = readFileSync(join(ui, "index.html"), "utf8");
const backend = readFileSync(join(ui, "..", "src-tauri", "src", "startup.rs"), "utf8");

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
