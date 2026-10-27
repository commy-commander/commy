import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  OVERLAY_MAX_LAYOUTS, OVERLAY_MAX_LAYOUT_NAME, OVERLAY_CHAT_LABEL, overlaySnap, overlayPart, overlayStartLayout, overlaySameLayout, overlayLayoutName, overlaySaveLayout,
} = require("../scripts/overlay-view.js");

const ui = join(import.meta.dirname, "..");
const rust = readFileSync(join(ui, "..", "src-tauri", "src", "overlay.rs"), "utf8");
const capability = JSON.parse(readFileSync(join(ui, "..", "src-tauri", "capabilities", "default.json"), "utf8"));

test("the app and the overlay windows agree on limits and names", () => {
  assert.match(rust, new RegExp(`MAX_LAYOUTS: usize = ${OVERLAY_MAX_LAYOUTS};`));
  assert.match(rust, new RegExp(`MAX_LAYOUT_NAME: usize = ${OVERLAY_MAX_LAYOUT_NAME};`));
  assert.match(rust, new RegExp(`CHAT_LABEL: &str = "${OVERLAY_CHAT_LABEL}";`));
  assert.ok(capability.windows.includes(OVERLAY_CHAT_LABEL));
});

test("a corner keeps voice and chat in one window, a layout splits them", () => {
  assert.deepEqual(overlayPart({ corner: "bottom-right" }, "overlay"), { part: "both", bottom: true, right: true });
  const layout = { voice: { x: 0.9, y: 0.1 }, chat: { x: 0.2, y: 0.8 } };
  assert.deepEqual(overlayPart({ layout }, "overlay"), { part: "voice", bottom: false, right: true });
  assert.deepEqual(overlayPart({ layout }, OVERLAY_CHAT_LABEL), { part: "chat", bottom: true, right: false });
});

test("dragging snaps to the edges and the middle", () => {
  assert.equal(overlaySnap(0.02), 0);
  assert.equal(overlaySnap(0.99), 1);
  assert.equal(overlaySnap(0.51), 0.5);
  assert.equal(overlaySnap(0.3333333), 0.333);
  assert.equal(overlaySnap(-4), 0);
  assert.equal(overlaySnap(Number.NaN), 0);
});

test("a fresh layout starts from the corner with chat on the other side", () => {
  assert.deepEqual(overlayStartLayout("top-right"), { voice: { x: 1, y: 0 }, chat: { x: 1, y: 1 } });
  assert.deepEqual(overlayStartLayout("bottom-left"), { voice: { x: 0, y: 1 }, chat: { x: 0, y: 0 } });
  assert.ok(overlaySameLayout(overlayStartLayout("top-left"), { voice: { x: 0, y: 0 }, chat: { x: 0, y: 1 } }));
  assert.ok(!overlaySameLayout(overlayStartLayout("top-left"), null));
});

test("saving a layout replaces one with the same name and keeps the newest few", () => {
  const layout = overlayStartLayout("top-left");
  let saved = [];
  for (let i = 0; i < OVERLAY_MAX_LAYOUTS + 3; i++) saved = overlaySaveLayout(saved, `Game ${i}`, layout);
  assert.equal(saved.length, OVERLAY_MAX_LAYOUTS);
  assert.equal(saved[0].name, "Game 3");
  saved = overlaySaveLayout(saved, "game 10", overlayStartLayout("top-right"));
  assert.equal(saved.filter(l => l.name.toLowerCase() === "game 10").length, 1);
  assert.equal(saved.at(-1).voice.x, 1);
  assert.equal(overlaySaveLayout(saved, "   ", layout), saved);
  assert.equal(overlayLayoutName(`  ${"x".repeat(50)}\u0007 `).length, OVERLAY_MAX_LAYOUT_NAME);
});
