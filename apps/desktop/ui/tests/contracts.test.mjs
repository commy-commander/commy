import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const read = (...path) => readFileSync(join(repo, ...path), "utf8");
const appSource = read("apps", "desktop", "ui", "scripts", "app.js");

test("the app loads images through the account server for exactly the hosts that server will fetch", () => {
  const appHosts = JSON.parse(/const MEDIA_HOSTS = new Set\((\[[^\]]*\])\)/.exec(appSource)[1]);
  const serverBlock = /const MEDIA_HOSTS = \{([\s\S]*?)\n\};/.exec(read("auth", "src", "media.js"))[1];
  const serverHosts = [...serverBlock.matchAll(/^\s*"([^"]+)":/gm)].map(m => m[1]);
  assert.deepEqual([...appHosts].sort(), [...serverHosts].sort());
});

test("built-in soundboard clips use the backend's ids, since peers play them by id", () => {
  const block = /const DEFAULT_SOUNDS = \[([\s\S]*?)\n\];/.exec(appSource)[1];
  const appIds = [...block.matchAll(/id: "([^"]+)"/g)].map(m => m[1]);
  const backend = /pub const DEFAULT_SOUNDS: &\[&str\] = &\[([^\]]*)\]/.exec(read("apps", "desktop", "src-tauri", "src", "expressions.rs"))[1];
  const backendIds = [...backend.matchAll(/"([^"]+)"/g)].map(m => m[1]);
  assert.deepEqual(appIds, backendIds);
});
