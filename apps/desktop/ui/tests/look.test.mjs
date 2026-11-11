import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { LOOK_KEY, LOOK_DEFAULT, resolveLook, lookFromSearch, windowFit, resolveFx, lookAttributes } = require("../scripts/look.js");
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

test("a window is slim up to 700 wide, roomy from 1180 by 760, compact in between", () => {
  assert.equal(windowFit(390, 844), "slim");
  assert.equal(windowFit(700, 2000), "slim");
  assert.equal(windowFit(701, 600), "compact");
  assert.equal(windowFit(1180, 759), "compact");
  assert.equal(windowFit(1179, 900), "compact");
  assert.equal(windowFit(1180, 760), "roomy");
  assert.equal(windowFit(1920, 1080), "roomy");
});

test("phones and slim windows start without glass, and a saved choice wins", () => {
  assert.equal(resolveFx(null, { fit: "slim", phoneApp: false }), "lite");
  assert.equal(resolveFx(null, { fit: "roomy", phoneApp: true }), "lite");
  assert.equal(resolveFx(null, { fit: "roomy", phoneApp: false }), "glass");
  assert.equal(resolveFx("glass", { fit: "slim", phoneApp: true }), "glass");
  assert.equal(resolveFx("lite", { fit: "roomy", phoneApp: false }), "lite");
});

test("the classic look sets none of the new attributes", () => {
  assert.deepEqual(lookAttributes({ stored: "classic", search: "", storedFx: "lite", width: 390, height: 844, phoneApp: true }), { look: null, fit: null, fx: null });
  assert.deepEqual(lookAttributes({ stored: null, search: "", storedFx: null, width: 1440, height: 900, phoneApp: false }), { look: "studio", fit: "roomy", fx: null });
  assert.deepEqual(lookAttributes({ stored: "studio", search: "", storedFx: null, width: 390, height: 844, phoneApp: true }), { look: "studio", fit: "slim", fx: "lite" });
  assert.deepEqual(lookAttributes({ stored: "studio", search: "", storedFx: null, width: 1440, height: 900, phoneApp: false }), { look: "studio", fit: "roomy", fx: null });
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
