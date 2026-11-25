import "./i18n-global.mjs";
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { LOOK_FIT, LOOK_STACK_KEY, tabletScreen, tabletFit, tabletPanes, windowFit, windowZoom, windowLayout, windowStack, stackChosen, zoomHop, knownZoom, resolveFx, lookAttributes } = require("../scripts/look.js");
const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");

test("the new look is the only look: nothing saved and nothing in the address turns it off", () => {
  const look = require("../scripts/look.js");
  for (const gone of ["LOOK_KEY", "LOOK_DEFAULT", "resolveLook", "lookFromSearch", "lookSearch", "chooseLook"]) assert.equal(look[gone], undefined, gone);
  assert.equal(lookAttributes({ stored: "classic", search: "?classic", storedFx: null, width: 1440, height: 900, phoneApp: false }).look, "studio");
  assert.doesNotMatch(html, /new-look-toggle/);
});

test("a window is slim up to 700 wide, where the old sheet's 700 media rules still fire, or at least twice as tall as wide, roomy from 1180 by 760, compact in between", () => {
  assert.equal(windowFit(390, 844), "slim");
  assert.equal(windowFit(699, 600), "slim");
  assert.equal(windowFit(700, 900), "slim");
  assert.equal(windowFit(700, 1399), "slim");
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
  const base = { storedFx: null };
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

test("every window gets the look, its fit and its effects", () => {
  assert.deepEqual(lookAttributes({ storedFx: "lite", width: 390, height: 844, phoneApp: true }), { look: "studio", fit: "slim", fx: "lite", stack: null });
  assert.deepEqual(lookAttributes({ storedFx: null, width: 1440, height: 900, phoneApp: false }), { look: "studio", fit: "roomy", fx: null, stack: null });
  assert.deepEqual(lookAttributes({ storedFx: null, width: 390, height: 844, phoneApp: true }), { look: "studio", fit: "slim", fx: "lite", stack: null });
});

test("a slim window wider than 550 zooms by its width over 550, and nothing else zooms", () => {
  assert.equal(windowZoom(1100, 3840), 2);
  assert.ok(Math.abs(windowZoom(733, 2560) - 733 / 550) < 1e-9);
  assert.ok(Math.abs(windowZoom(650, 900) - 650 / 550) < 1e-9);
  assert.equal(windowZoom(550, 1920), 1);
  assert.equal(windowZoom(420, 800), 1);
  assert.ok(Math.abs(windowZoom(700, 900) - 700 / 550) < 1e-9);
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

test("a tall slim desktop window keeps the phone layout unless the stacked view is chosen", () => {
  const base = { storedFx: null, phoneApp: false };
  assert.deepEqual(lookAttributes({ ...base, width: 1100, height: 3840 }), { look: "studio", fit: "slim", fx: "lite", stack: null });
  assert.deepEqual(lookAttributes({ ...base, storedStack: "off", width: 1100, height: 3840 }), { look: "studio", fit: "slim", fx: "lite", stack: null });
  assert.deepEqual(lookAttributes({ ...base, width: 578, height: 2000 }), { look: "studio", fit: "slim", fx: "lite", stack: null });
  assert.deepEqual(lookAttributes({ ...base, width: 701, height: 900 }), { look: "studio", fit: "compact", fx: null, stack: null });
  assert.equal(LOOK_STACK_KEY, "commy.tallStack");
  assert.equal(stackChosen("on"), true);
  assert.equal(stackChosen("off"), false);
  assert.equal(stackChosen(null), false);
});

test("with the stacked view chosen, a slim window stacks when its height over the zoom is 1800 or more", () => {
  assert.equal(windowStack(1100, 3840), true);
  assert.equal(windowStack(1100, 3599), false);
  assert.equal(windowStack(550, 1800), true);
  assert.equal(windowStack(550, 1799), false);
  assert.equal(windowStack(420, 800), false);
  assert.equal(windowStack(1920, 4000), false);
  const base = { storedFx: null, storedStack: "on", phoneApp: false };
  assert.deepEqual(lookAttributes({ ...base, width: 1100, height: 3840 }), { look: "studio", fit: "slim", fx: "lite", stack: "on" });
  assert.deepEqual(lookAttributes({ ...base, width: 550, height: 1920, zoom: 2 }), { look: "studio", fit: "slim", fx: "lite", stack: "on" });
  assert.deepEqual(lookAttributes({ ...base, width: 578, height: 2000 }), { look: "studio", fit: "slim", fx: "lite", stack: "on" });
  assert.deepEqual(lookAttributes({ ...base, width: 1100, height: 2400 }), { look: "studio", fit: "slim", fx: "lite", stack: null });
  assert.deepEqual(lookAttributes({ ...base, width: 420, height: 800 }), { look: "studio", fit: "slim", fx: "lite", stack: null });
  assert.deepEqual(lookAttributes({ ...base, phoneApp: true, width: 1100, height: 3840 }), { look: "studio", fit: "slim", fx: "lite", stack: null });
});

test("Appearance has a Stacked Tall Window switch that phones do not show", () => {
  assert.ok(html.includes('<label class="settings-card setting-row desktop-only" data-search="tall window stacked stack layout narrow portrait">'));
  assert.ok(html.includes('<input type="checkbox" id="tall-stack-toggle" class="switch" />'));
  const app = readFileSync(new URL("../scripts/app.js", import.meta.url), "utf8");
  assert.ok(app.includes('$("tall-stack-toggle").checked = lookStackChosen();'));
  assert.ok(app.includes("e.target.checked = setLookStack(e.target.checked);"));
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

test("a screen whose short side is 600 or more is a tablet", () => {
  assert.equal(tabletScreen(834, 1194), true);
  assert.equal(tabletScreen(1194, 834), true);
  assert.equal(tabletScreen(600, 960), true);
  assert.equal(tabletScreen(599, 960), false);
  assert.equal(tabletScreen(390, 844), false);
  assert.equal(tabletScreen(844, 390), false);
  assert.equal(tabletScreen(undefined, undefined), false);
});

test("a tablet in the phone app gets two panes under 1024 wide and three from 1024, upright or on its side", () => {
  assert.equal(tabletFit(834, 1194), "compact");
  assert.equal(tabletFit(704, 1001), "compact");
  assert.equal(tabletFit(700, 1000), "slim");
  assert.equal(tabletFit(1194, 834), "roomy");
  assert.equal(tabletFit(1024, 700), "compact");
  assert.equal(tabletPanes(704), "2");
  assert.equal(tabletPanes(1023), "2");
  assert.equal(tabletPanes(1024), "3");
  assert.equal(windowFit(800, 1280, true, true), "compact");
  assert.equal(windowFit(1024, 1366, true, true), "compact");
  assert.equal(windowFit(1280, 800, true, true), "roomy");
  const base = { storedFx: null, phoneApp: true, tablet: true };
  assert.deepEqual(lookAttributes({ ...base, width: 834, height: 1194 }), { look: "studio", fit: "compact", fx: "lite", stack: null, touch: "tablet", panes: "2" });
  assert.deepEqual(lookAttributes({ ...base, width: 1194, height: 834 }), { look: "studio", fit: "roomy", fx: "lite", stack: null, touch: "tablet", panes: "3" });
  assert.deepEqual(lookAttributes({ ...base, width: 1024, height: 1366 }), { look: "studio", fit: "compact", fx: "lite", stack: null, touch: "tablet", panes: "3" });
  assert.deepEqual(lookAttributes({ ...base, width: 550, height: 900 }), { look: "studio", fit: "slim", fx: "lite", stack: null });
  assert.deepEqual(lookAttributes({ ...base, phoneApp: false, width: 834, height: 1194 }), { look: "studio", fit: "compact", fx: null, stack: null });
});

test("tablet touch sizes load after the desktop sheets and only apply to a tablet", () => {
  const css = readFileSync(new URL("../styles/tablet.css", import.meta.url), "utf8");
  assert.ok(html.indexOf('id="studio-tablet"') > html.indexOf('href="styles/studio.css"'));
  for (const selector of css.match(/^[^@\s}][^{]*(?=\{)/gm)) for (const part of selector.split(",")) assert.match(part.trim(), /^html\[data-look="studio"\]\[data-touch="tablet"\]/, part);
  assert.match(css, /\.channel \{ min-height: 44px;/);
  assert.match(css, /\.composer textarea \{ font-size: 16px; \}/);
});
