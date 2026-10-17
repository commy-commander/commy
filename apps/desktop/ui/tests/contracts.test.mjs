import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const read = (...path) => readFileSync(join(repo, ...path), "utf8");
const appSource = read("apps", "desktop", "ui", "scripts", "app.js");

test("built-in soundboard clips use the backend's ids, since peers play them by id", () => {
  const block = /const DEFAULT_SOUNDS = \[([\s\S]*?)\n\];/.exec(appSource)[1];
  const appIds = [...block.matchAll(/id: "([^"]+)"/g)].map(m => m[1]);
  const backend = /pub const DEFAULT_SOUNDS: &\[&str\] = &\[([^\]]*)\]/.exec(read("apps", "desktop", "src-tauri", "src", "expressions.rs"))[1];
  const backendIds = [...backend.matchAll(/"([^"]+)"/g)].map(m => m[1]);
  assert.deepEqual(appIds, backendIds);
});
