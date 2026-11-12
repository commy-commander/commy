import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { OVERLAY_STAR, overlayPicture, overlayTestMember, overlayTileKey } = require("../scripts/overlay-view.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const page = read("../overlay.html");
const css = read("../styles/studio-overlay.css");
const script = read("../scripts/overlay.js");
const app = read("../scripts/app.js");

const SWITCH = 'html[data-look="studio"]';
const FRAMES = /@keyframes[^{]+\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g;
const selectors = sheet => [...sheet.replace(FRAMES, "").replace(/@media[^{]+\{/g, "").matchAll(/(^|\})\s*([^{}@]+)\{/g)].flatMap(match => match[2].split(",").map(part => part.trim())).filter(Boolean);
const rule = selector => {
  const start = css.indexOf(`${SWITCH} ${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("the overlay's new look stays off until the switch is on", () => {
  const all = selectors(css);
  assert.ok(all.length > 30);
  for (const selector of all) assert.ok(selector.startsWith(SWITCH), selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  assert.match(page, /<link rel="stylesheet" id="studio-tokens" href="styles\/tokens\.css" media="not all" \/>/);
  assert.match(page, /<link rel="stylesheet" id="studio-overlay" href="styles\/studio-overlay\.css" media="not all" \/>/);
  const order = ["scripts/accessibility.js", "scripts/look.js", "scripts/studio-faces.js", "scripts/overlay-view.js", "scripts/overlay.js"].map(name => page.indexOf(`<script src="${name}"></script>`));
  assert.ok(order.every(at => at > 0));
  assert.deepEqual([...order].sort((a, b) => a - b), order);
  assert.match(script, /if \(e\.key === LOOK_FX_KEY\) \{\n\s+applyLook\(\);\n\s+renderMembers\(\);/);
});

test("a pane over a game is smoked colour, never a blur", () => {
  assert.equal(css.includes("backdrop-filter"), false);
  assert.match(css, /--ov-pane: rgb\(27 30 39 \/ \.9\);/);
  assert.match(css, /--ov-pane-quiet: rgb\(27 30 39 \/ \.74\);/);
  assert.match(css, /html\[data-look="studio"\]\[data-theme="light"\] \{\n  --ov-pane: rgb\(255 255 255 \/ \.93\);/);
  assert.match(rule(".member"), /background: var\(--ov-pane-quiet\);/);
  assert.match(rule(".member"), /box-shadow: var\(--ov-rim\);/);
  assert.match(rule(".member.speaking"), /background: var\(--ov-lamp\), var\(--ov-pane\);/);
  for (const part of [".line", ".notice"]) {
    assert.match(rule(part), /background: var\(--ov-lamp\), var\(--ov-pane\);/);
    assert.match(rule(part), /box-shadow: var\(--ov-rim\), var\(--ov-drop\);/);
  }
});

test("whoever talks gets the full pane, a bright name and a steady green ring", () => {
  assert.match(rule(".member"), /height: 32px;/);
  assert.match(rule(".member"), /opacity: 1;/);
  assert.match(rule(".member .avatar"), /width: 24px;\n  height: 24px;/);
  assert.match(rule(".member .avatar"), /opacity: \.78;/);
  assert.match(rule(".member.speaking .avatar"), /box-shadow: 0 0 0 2px var\(--ov-speak\); opacity: 1;/);
  assert.match(rule(".name"), /font: 500 14px\/1 "Onest", system-ui, sans-serif;\n  color: var\(--ov-quiet\);/);
  assert.match(rule(".member.speaking .name"), /font-weight: 600; color: var\(--ov-ink\);/);
  assert.doesNotMatch(css, /infinite/);
});

test("pictures only is a round pane of 36 with a picture of 28", () => {
  assert.match(rule("body.pictures #members"), /max-width: 246px; gap: 6px;/);
  assert.match(rule("body.pictures .member"), /width: 36px; height: 36px; padding: 0;/);
  assert.match(rule("body.pictures .member .avatar"), /width: 28px; height: 28px;/);
});

test("messages and notices are set in one family at 14, and the notice wears the star", () => {
  assert.match(rule(".line"), /max-width: 280px;/);
  assert.match(rule(".line"), /border-radius: 17px;/);
  assert.match(rule(".line"), /font: 400 14px\/1\.4 "Onest", system-ui, sans-serif;/);
  assert.match(rule(".notice"), /height: 32px;/);
  assert.match(rule(".notice"), /font: 600 14px\/1 "Onest", system-ui, sans-serif;/);
  assert.match(rule(".notice::before"), /content: none;/);
  assert.match(rule(".notice .ov-star"), /width: 20px; height: 20px;/);
  assert.equal(css.includes("Funnel Display"), false);
  assert.equal(OVERLAY_STAR.length, 3);
  for (const [fill, d] of OVERLAY_STAR) {
    assert.match(fill, /^#[0-9a-f]{6}$/);
    assert.match(d, /^M[\d.]+ [\d.]+/);
  }
  assert.match(script, /if \(studioLook\(\)\) notice\.prepend\(starNode\(\)\);/);
});

test("parts arrive in 160 ms and hold still when motion is reduced", () => {
  assert.match(css, /@keyframes ov-in \{ from \{ opacity: 0; translate: 0 4px; \} \}/);
  for (const part of [".member", ".line", ".notice"]) assert.match(rule(part), /animation: ov-in \.16s cubic-bezier\(\.2, \.7, \.2, 1\);/);
  assert.match(css, /html\[data-look="studio"\]\[data-reduced-motion\] \.member,\nhtml\[data-look="studio"\]\[data-reduced-motion\] \.line,\nhtml\[data-look="studio"\]\[data-reduced-motion\] \.notice \{ animation: none; transition: none; \}/);
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{\n  html\[data-look="studio"\] \.member,\n  html\[data-look="studio"\] \.line,\n  html\[data-look="studio"\] \.notice \{ animation: none; transition: none; \}\n\}/);
});

test("a person with no picture gets a face in the new look and initials in the old one", () => {
  assert.deepEqual(overlayPicture({ id: "m1", name: "ferris wheel", image: "data:image/png;base64,AA" }, true), { kind: "image", value: "data:image/png;base64,AA" });
  assert.deepEqual(overlayPicture({ id: "m1", name: "ferris", emoji: "🦀" }, true), { kind: "emoji", value: "🦀" });
  assert.deepEqual(overlayPicture({ id: "m1", name: "ferris wheel" }, true), { kind: "face", value: "m1" });
  assert.deepEqual(overlayPicture({ id: "overlay-test", name: "You, talking", face: "me" }, true), { kind: "face", value: "me" });
  assert.deepEqual(overlayPicture({ id: "m1", name: "ferris wheel" }, false), { kind: "initials", value: "FW" });
  assert.deepEqual(overlayPicture({ id: "m1", name: "" }, false), { kind: "initials", value: "?" });
  assert.match(script, /avatar\.appendChild\(studioFaceNode\(document, picture\.value\)\);/);
});

test("the test tile wears your own picture in the new look", () => {
  const plain = { id: "overlay-test", name: "You, talking" };
  assert.deepEqual(overlayTestMember(null, false), plain);
  assert.deepEqual(overlayTestMember({ id: "me", emoji: "🦀", image: null }, false), plain);
  assert.deepEqual(overlayTestMember(null, true), plain);
  assert.deepEqual(overlayTestMember({ id: "me", emoji: "🦀", image: null }, true), { ...plain, face: "me", image: null, emoji: "🦀" });
  assert.deepEqual(overlayTestMember({ id: "me" }, true), { ...plain, face: "me", image: null, emoji: null });
  assert.match(app, /emitToOverlay\("overlay-test", state\.user \? \{ id: state\.user\.id, \.\.\.overlayPictureOf\(state\.user\.id\) \} : null\)/);
  assert.match(script, /listen\("overlay-test", e => runTest\(e\.payload\), own\);/);
});

test("a tile is redrawn when the look changes, and the old look's key is as before", () => {
  const nova = { id: "nova", name: "nova" };
  assert.equal(overlayTileKey(nova, false), JSON.stringify(["nova", "", "", false]));
  assert.equal(overlayTileKey(nova, false, false), overlayTileKey(nova, false));
  assert.notEqual(overlayTileKey(nova, false, true), overlayTileKey(nova, false));
  assert.notEqual(overlayTileKey({ ...nova, face: "me" }, false, true), overlayTileKey(nova, false, true));
});

test("message authors keep their role colour at Night and a deeper mix of it in Day", () => {
  assert.match(css, /html\[data-look="studio"\] \.line b \{[^}]*color: var\(--ov-role, var\(--ov-ink\)\); \}/);
  assert.match(css, /html\[data-look="studio"\]\[data-theme="light"\] \.line b \{ color: color-mix\(in oklab, var\(--ov-role, var\(--ov-ink\)\) 55%, var\(--ov-ink\)\); \}/);
  assert.match(script, /if \(line\.color && studio\) who\.style\.setProperty\("--ov-role", `#\$\{line\.color\}`\);\n  else if \(line\.color\) who\.style\.color = `#\$\{line\.color\}`;/);
});
