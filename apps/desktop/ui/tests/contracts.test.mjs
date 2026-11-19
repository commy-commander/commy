import { englishSource } from "./i18n-global.mjs";
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const read = (...path) => englishSource(readFileSync(join(repo, ...path), "utf8"));
const appSource = read("apps", "desktop", "ui", "scripts", "app.js");

test("built-in soundboard clips use the backend's ids and labels, since peers play them by id", () => {
  const block = /const DEFAULT_SOUNDS = \[([\s\S]*?)\n\];/.exec(appSource)[1];
  const appSounds = [...block.matchAll(/id: "([^"]+)", label: "([^"]+)"/g)].map(m => [m[1], m[2]]);
  const backend = /pub const DEFAULT_SOUNDS: &\[DefaultSound\] = &\[([\s\S]*?)\n\];/.exec(read("crates", "commy-view", "src", "expressions.rs"))[1];
  const backendSounds = [...backend.matchAll(/id: "([^"]+)", label: "([^"]+)"/g)].map(m => [m[1], m[2]]);
  assert.equal(appSounds.length, 4);
  assert.deepEqual(appSounds, backendSounds);
});

test("the sound upload limits match the backend, which refuses anything larger", () => {
  const trim = read("apps", "desktop", "ui", "scripts", "sound-trim.js");
  const limit = (source, name) => {
    const m = new RegExp(`${name}(?::\\s*usize)?\\s*=\\s*([\\d\\s*]+);`).exec(source);
    return m[1].split("*").reduce((product, n) => product * Number(n), 1);
  };
  assert.equal(limit(trim, "SOUND_STORE_MAX_BYTES"), limit(read("crates", "commy-view", "src", "pictures.rs"), "MAX_SOUND_BYTES"));
  assert.equal(limit(trim, "SOUND_SOURCE_MAX_BYTES"), limit(read("apps", "desktop", "src-tauri", "src", "expressions.rs"), "MAX_SOUND_SOURCE_BYTES"));
});

test("the app and the network agree on which files download straight to disk", () => {
  const limit = (source, name) => {
    const m = new RegExp(`${name}(?::\\s*u64)?\\s*=\\s*([\\d\\s*]+);`).exec(source);
    return m[1].split("*").reduce((product, n) => product * Number(n), 1);
  };
  const ui = read("apps", "desktop", "ui", "scripts", "embeds.js");
  const net = read("crates", "commy-net", "src", "node.rs");
  assert.equal(limit(ui, "BIG_FILE_BYTES"), limit(net, "BIG_FILE_BYTES"));
});

test("the app and the backend agree on the largest video poster", () => {
  const limit = (source, name) => {
    const m = new RegExp(`${name}(?::\\s*usize)?\\s*=\\s*([\\d\\s*]+);`).exec(source);
    return m[1].split("*").reduce((product, n) => product * Number(n), 1);
  };
  const ui = read("apps", "desktop", "ui", "scripts", "embeds.js");
  const core = read("crates", "commy-core", "src", "video_poster.rs");
  assert.equal(limit(ui, "MAX_POSTER_BYTES"), limit(core, "MAX_VIDEO_POSTER_BYTES"));
});

test("confirm boxes wait for an answer, since the phone apps answer them later and need permission to show them", () => {
  const confirms = [...appSource.matchAll(/(await\s+)?\bconfirm\(/g)];
  assert.ok(confirms.length > 0);
  assert.deepEqual(confirms.filter(m => !m[1]).map(m => appSource.slice(m.index, m.index + 60)), []);
  const mobile = JSON.parse(read("apps", "desktop", "src-tauri", "capabilities", "mobile.json"));
  assert.deepEqual(mobile.platforms.sort(), ["android", "iOS"]);
  assert.ok(mobile.permissions.includes("dialog:allow-message"));
});

test("the removed-by-sync-server notice reads the same as the refusal the backend sends", () => {
  const quoted = (source, pattern) => new RegExp(pattern).exec(source)[1];
  const ui = quoted(appSource, 'const HOST_REMOVED_TEXT = "([^"]+)";');
  const backend = quoted(read("crates", "commy-view", "src", "sync_host.rs"), 'pub const HOST_REMOVED: &str = "([^"]+)";');
  assert.equal(ui, backend);
});

test("the app listens for every way a server can put someone out", () => {
  for (const notice of ["banned", "kicked", "home-blocked", "host-removed", "server-deleted"]) {
    assert.ok(appSource.includes(`event.listen("${notice}"`), notice);
  }
  const desktop = read("apps", "desktop", "src-tauri", "src", "lib.rs");
  assert.ok(desktop.includes('const HOME_BLOCKED: &str = "home-blocked";'));
  const web = read("apps", "web", "src", "client.rs");
  assert.ok(web.includes('"home-blocked"'));
});
