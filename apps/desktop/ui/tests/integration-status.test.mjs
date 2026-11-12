import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";

const require = createRequire(import.meta.url);
const { INTEGRATION_FRESH_MS, INTEGRATION_KINDS, INTEGRATION_WORDS, integrationStatus, createIntegrationChecks } = require("../scripts/integration-status.js");
const read = name => readFileSync(new URL(`../${name}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("index.html");
const app = read("scripts/app.js");
const css = read("styles/styles.css");

test("a connected integration says whether it works, with the reason when it does not", () => {
  for (const kind of INTEGRATION_KINDS) {
    assert.deepEqual(integrationStatus(kind, { configured: true, ok: true }), { tone: "ok", text: "Connected and working" }, kind);
    assert.deepEqual(integrationStatus(kind, { configured: true, ok: false, reason: "KLIPY answered 403." }), { tone: "warn", text: "Connected, but not working: KLIPY answered 403" }, kind);
    assert.deepEqual(integrationStatus(kind, { configured: true, ok: false, reason: "empty" }), { tone: "warn", text: "Connected, but not working" }, kind);
    assert.deepEqual(integrationStatus(kind, null), { tone: "busy", text: "Checking the connection" }, kind);
  }
});

test("without a key the built-in searches say when they work and when nothing is connected", () => {
  assert.deepEqual(integrationStatus("gif-search", { configured: false, ok: true }), { tone: "ok", text: INTEGRATION_WORDS["gif-search"].builtin });
  assert.deepEqual(integrationStatus("gif-search", { configured: false, ok: false, reason: "no-key" }), { tone: "off", text: INTEGRATION_WORDS["gif-search"].off });
  assert.deepEqual(integrationStatus("gif-search", { configured: false, ok: false, reason: "timed out" }), { tone: "warn", text: "The built-in search is not working: timed out" });
  assert.deepEqual(integrationStatus("game-database", { configured: false, ok: true }), { tone: "ok", text: INTEGRATION_WORDS["game-database"].builtin });
  assert.deepEqual(integrationStatus("game-database", { configured: false, ok: false, reason: "empty" }), { tone: "off", text: INTEGRATION_WORDS["game-database"].off });
});

test("file storage is off until a worker is connected and warns when the worker is not set up", () => {
  assert.deepEqual(integrationStatus("file-storage", { configured: false, ok: false }), { tone: "off", text: "Not connected" });
  assert.deepEqual(integrationStatus("file-storage", { configured: false, ok: true }), { tone: "off", text: "Not connected" });
  assert.equal(integrationStatus("file-storage", { configured: true, ok: false, reason: "unpaired" }).text, INTEGRATION_WORDS["file-storage"].unpaired);
  assert.equal(integrationStatus("file-storage", { configured: true, ok: false, reason: "unpaired" }).tone, "warn");
  assert.deepEqual(integrationStatus("bots", { configured: true, ok: true }), { tone: "off", text: "" });
});

test("a check is remembered for ten minutes and forgotten after a save", () => {
  let clock = 1000;
  const checks = createIntegrationChecks(() => clock);
  const status = { tone: "ok", text: "Connected and working" };
  assert.equal(checks.fresh("a"), null);
  assert.equal(checks.keep("a", status), status);
  clock += INTEGRATION_FRESH_MS - 1;
  assert.equal(checks.fresh("a"), status);
  clock += 1;
  assert.equal(checks.fresh("a"), null);
  checks.keep("a", status);
  checks.clear();
  assert.equal(checks.fresh("a"), null);
});

test("each integration page has a status row, a tab and a way to check again", () => {
  for (const kind of INTEGRATION_KINDS) {
    assert.ok(html.includes(`<div class="integration-status" id="${kind}-status" data-tone="off" role="status">`), kind);
    assert.ok(html.includes(`<button class="btn-link integration-recheck" type="button" data-kind="${kind}">Check Again</button>`), kind);
    assert.match(html, new RegExp(`data-tab="${kind}" id="tab-${kind}"`), kind);
    assert.ok(app.includes(`checkIntegration("${kind}");`), kind);
  }
  assert.ok(html.indexOf('<script src="scripts/integration-status.js"></script>') < html.indexOf('<script src="scripts/app.js"></script>'));
  assert.equal(app.split("integrationChecks.clear();").length - 1, 3);
  assert.ok(app.includes('again.onclick = () => checkIntegration(again.dataset.kind, true);'));
  assert.ok(css.includes('.integration-status[data-tone="ok"] .integration-dot, .tab-dot[data-tone="ok"] { background: var(--green); }'));
  assert.ok(css.includes('.integration-status[data-tone="warn"] .integration-dot, .tab-dot[data-tone="warn"] { background: var(--accent-2); }'));
});
