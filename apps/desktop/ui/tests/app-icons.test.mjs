import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { APP_ICON_PRESETS, appIconChoices } = require("../scripts/app-icons.js");

test("the tray can follow the app icon, the app icon cannot", () => {
  assert.equal(appIconChoices("tray", false)[0].id, "app");
  assert.ok(!appIconChoices("app", false).some(t => t.id === "app"));
});

test("both pickers end with the person's own image", () => {
  for (const target of ["app", "tray"]) {
    const last = appIconChoices(target, true).at(-1);
    assert.deepEqual([last.id, last.label, last.custom], ["custom", "Yours", true]);
    assert.equal(appIconChoices(target, false).at(-1).label, "Your own");
  }
});

test("the presets offered are the ones the desktop app can build", () => {
  const source = readFileSync(join(import.meta.dirname, "..", "..", "src-tauri", "src", "app_icon.rs"), "utf8");
  const list = /pub const PRESETS: &\[&str\] = &\[([^\]]*)\]/.exec(source)[1];
  const names = [...list.matchAll(/"([a-z]+)"/g)].map(m => m[1]);
  const ids = APP_ICON_PRESETS.map(p => p.id);
  assert.ok(source.includes('pub const CLASSIC: &str = "classic"'));
  assert.deepEqual(ids, ["classic", ...names]);
});
