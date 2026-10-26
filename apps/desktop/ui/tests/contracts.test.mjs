import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const read = (...path) => readFileSync(join(repo, ...path), "utf8");
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
