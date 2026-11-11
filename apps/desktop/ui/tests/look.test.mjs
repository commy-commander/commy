import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { LOOK_KEY, LOOK_DEFAULT, LOOK_FIT, resolveLook, lookFromSearch, windowFit, windowZoom, windowLayout, windowStack, zoomHop, knownZoom, resolveFx, lookAttributes } = require("../scripts/look.js");
const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");

test("the new look is on until someone turns it off", () => {
  assert.equal(LOOK_DEFAULT, "studio");
  assert.equal(resolveLook(null, ""), "studio");
  assert.equal(resolveLook("anything", ""), "studio");
  assert.equal(resolveLook("studio", ""), "studio");
  assert.equal(resolveLook("classic", ""), "classic");
});

test("a classic choice saved while the new look was opt-in is not carried over", () => {
  assert.equal(LOOK_KEY, "commy.look.2");
});

test("the address can ask for either look, and wins over the saved choice", () => {
  assert.equal(lookFromSearch("?mobile&studio"), "studio");
  assert.equal(lookFromSearch("?studio=1"), "studio");
  assert.equal(lookFromSearch("?mobile"), null);
  assert.equal(lookFromSearch("?studious"), null);
  assert.equal(resolveLook("classic", "?studio"), "studio");
  assert.equal(resolveLook("studio", "?classic"), "classic");
});

test("a window is slim under 700 wide or at least twice as tall as wide, roomy from 1180 by 760, compact in between", () => {
  assert.equal(windowFit(390, 844), "slim");
  assert.equal(windowFit(699, 600), "slim");
  assert.equal(windowFit(700, 900), "compact");
  assert.equal(windowFit(700, 1399), "compact");
  assert.equal(windowFit(700, 1400), "slim");
  assert.equal(windowFit(701, 600), "compact");
  assert.equal(windowFit(1100, 3840), "slim");
  assert.equal(windowFit(1100, 2199), "compact");
  assert.equal(windowFit(1100, 2200), "slim");
  assert.equal(windowFit(1180, 759), "compact");
  assert.equal(windowFit(1179, 900), "compact");
  assert.equal(windowFit(1180, 760), "roomy");
  assert.equal(windowFit(1920, 1080), "roomy");
  assert.equal(windowFit(1080, 1920), "compact");
});

test("in the phone app a screen taller than wide is slim, and one on its side keeps the desktop fits", () => {
  assert.equal(windowFit(800, 1280, true), "slim");
  assert.equal(windowFit(1024, 1366, true), "slim");
  assert.equal(windowFit(1280, 800, true), "roomy");
  assert.equal(windowFit(1000, 700, true), "compact");
  assert.equal(windowFit(800, 800, true), "compact");
  assert.equal(windowFit(800, 1280, false), "compact");
  const base = { stored: "studio", search: "", storedFx: null };
  assert.deepEqual(lookAttributes({ ...base, width: 800, height: 1280, phoneApp: true }), { look: "studio", fit: "slim", fx: "lite", stack: null });
  assert.deepEqual(lookAttributes({ ...base, width: 1280, height: 800, phoneApp: true }), { look: "studio", fit: "roomy", fx: "lite", stack: null });
  assert.deepEqual(lookAttributes({ ...base, width: 1100, height: 3840, phoneApp: true }), { look: "studio", fit: "slim", fx: "lite", stack: null });
});

test("phones and slim windows start without glass, and a saved choice wins", () => {
  assert.equal(resolveFx(null, { fit: "slim", phoneApp: false }), "lite");
  assert.equal(resolveFx(null, { fit: "roomy", phoneApp: true }), "lite");
  assert.equal(resolveFx(null, { fit: "roomy", phoneApp: false }), "glass");
  assert.equal(resolveFx("glass", { fit: "slim", phoneApp: true }), "glass");
  assert.equal(resolveFx("lite", { fit: "roomy", phoneApp: false }), "lite");
});

test("the classic look sets none of the new attributes", () => {
  assert.deepEqual(lookAttributes({ stored: "classic", search: "", storedFx: "lite", width: 390, height: 844, phoneApp: true }), { look: null, fit: null, fx: null, stack: null });
  assert.deepEqual(lookAttributes({ stored: null, search: "", storedFx: null, width: 1440, height: 900, phoneApp: false }), { look: "studio", fit: "roomy", fx: null, stack: null });
  assert.deepEqual(lookAttributes({ stored: "studio", search: "", storedFx: null, width: 390, height: 844, phoneApp: true }), { look: "studio", fit: "slim", fx: "lite", stack: null });
  assert.deepEqual(lookAttributes({ stored: "studio", search: "", storedFx: null, width: 1440, height: 900, phoneApp: false }), { look: "studio", fit: "roomy", fx: null, stack: null });
});

test("a slim window wider than 550 zooms by its width over 550, and nothing else zooms", () => {
  assert.equal(windowZoom(1100, 3840), 2);
  assert.ok(Math.abs(windowZoom(733, 2560) - 733 / 550) < 1e-9);
  assert.ok(Math.abs(windowZoom(650, 900) - 650 / 550) < 1e-9);
  assert.equal(windowZoom(550, 1920), 1);
  assert.equal(windowZoom(420, 800), 1);
  assert.equal(windowZoom(700, 900), 1);
  assert.ok(Math.abs(windowZoom(700, 1400) - 700 / 550) < 1e-9);
  assert.equal(windowZoom(1100, 2199), 1);
  assert.equal(windowZoom(1100, 2200), 2);
  assert.equal(windowZoom(1920, 1080), 1);
  assert.equal(windowZoom(NaN, 100), 1);
  assert.equal(windowZoom(9000, 20000), LOOK_FIT.zoomMax);
});

test("the layout is worked out from the real window, whatever zoom is on now", () => {
  assert.deepEqual(windowLayout({ width: 1100, height: 3840, zoom: 1, canZoom: true }), { zoom: 2, width: 550, height: 1920 });
  assert.deepEqual(windowLayout({ width: 550, height: 1920, zoom: 2, canZoom: true }), { zoom: 2, width: 550, height: 1920 });
  assert.deepEqual(windowLayout({ width: 800, height: 500, zoom: 2, canZoom: true }), { zoom: 1, width: 1600, height: 1000 });
  assert.deepEqual(windowLayout({ width: 1100, height: 3840, zoom: 1, canZoom: false }), { zoom: 1, width: 1100, height: 3840 });
  assert.deepEqual(windowLayout({ width: 1100, height: 3840 }), { zoom: 1, width: 1100, height: 3840 });
});

test("a slim window stacks when its height over the zoom is 1800 or more", () => {
  assert.equal(windowStack(1100, 3840), true);
  assert.equal(windowStack(1100, 3599), false);
  assert.equal(windowStack(1100, 3600), true);
  assert.equal(windowStack(733, 2560), true);
  assert.equal(windowStack(550, 1800), true);
  assert.equal(windowStack(550, 1799), false);
  assert.equal(windowStack(420, 800), false);
  assert.equal(windowStack(1920, 4000), false);
  const base = { stored: "studio", search: "", storedFx: null, phoneApp: false };
  assert.deepEqual(lookAttributes({ ...base, width: 1100, height: 3840 }), { look: "studio", fit: "slim", fx: "lite", stack: "on" });
  assert.deepEqual(lookAttributes({ ...base, width: 550, height: 1920, zoom: 2 }), { look: "studio", fit: "slim", fx: "lite", stack: "on" });
  assert.deepEqual(lookAttributes({ ...base, width: 550, height: 1920 }), { look: "studio", fit: "slim", fx: "lite", stack: "on" });
  assert.deepEqual(lookAttributes({ ...base, width: 1100, height: 2400 }), { look: "studio", fit: "slim", fx: "lite", stack: null });
  assert.deepEqual(lookAttributes({ ...base, width: 420, height: 800 }), { look: "studio", fit: "slim", fx: "lite", stack: null });
  assert.deepEqual(lookAttributes({ ...base, width: 700, height: 900 }), { look: "studio", fit: "compact", fx: null, stack: null });
});

test("the zoom moves only when it must, gives up after three moves in a row, and a bad saved zoom is ignored", () => {
  assert.equal(zoomHop(2, 2, 0), "stay");
  assert.equal(zoomHop(2, 2.004, 2), "stay");
  assert.equal(zoomHop(1, 2, 0), "move");
  assert.equal(zoomHop(2, 1, 2), "move");
  assert.equal(zoomHop(2, 4, 3), "give-up");
  assert.equal(knownZoom("2"), 2);
  assert.equal(knownZoom(null), 1);
  assert.equal(knownZoom("0.5"), 1);
  assert.equal(knownZoom("99"), 1);
  assert.equal(knownZoom("x"), 1);
  const capability = JSON.parse(readFileSync(new URL("../../src-tauri/capabilities/default.json", import.meta.url), "utf8"));
  assert.ok(capability.permissions.includes("core:webview:allow-set-webview-zoom"));
});

test("the phone app never zooms the webview, and look.js looks again once the phone app is known", () => {
  const look = readFileSync(new URL("../scripts/look.js", import.meta.url), "utf8");
  assert.match(look, /lookWindow\.broken \|\| document\.documentElement\.classList\.contains\("mobile-app"\)/);
  assert.match(look, /attributeFilter: \["class"\]/);
});

test("every new-look stylesheet is linked switched off, after the main one, and the look script is not the first script", () => {
  const links = [...html.matchAll(/<link rel="stylesheet" id="(studio-[a-z-]+)" href="([^"]+)" media="([^"]+)" \/>/g)];
  assert.ok(links.length >= 2);
  for (const [, id, , media] of links) assert.equal(media, "not all", id);
  assert.ok(links.some(([, id, href]) => id === "studio-tokens" && href === "styles/tokens.css"));
  assert.ok(html.indexOf('href="styles/styles.css"') < html.indexOf('id="studio-tokens"'));
  assert.match(html, /^  <link rel="stylesheet" href="styles\/styles\.css" \/>$/m);
  const first = html.indexOf('<script src="scripts/');
  assert.ok(first >= 0 && first < html.indexOf('<script src="scripts/look.js"'));
  assert.ok(html.indexOf('<script src="scripts/look.js"') < html.indexOf('<script src="scripts/app.js"'));
});
