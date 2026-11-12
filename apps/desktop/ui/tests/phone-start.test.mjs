import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-start.css");
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';

const rule = selector => {
  const at = css.indexOf(`${SLIM}${selector} {`);
  assert.notEqual(at, -1, selector);
  return css.slice(at, css.indexOf("}", at));
};

test("every phone startup rule is for the phone layout of the new look, with tokens and no comments", () => {
  const selectors = css.split("}").map(block => block.split("{").at(-2) || "").flatMap(head => head.replace(/\([^()]*\)/g, "()").replace(/\([^()]*\)/g, "()").split(",")).map(s => s.trim()).filter(Boolean);
  const plain = selectors.filter(s => !/^(@keyframes|@media|from|to|\d+%)/.test(s));
  assert.ok(plain.length > 25);
  for (const selector of plain) assert.ok(selector.startsWith(SLIM), selector);
  assert.equal(/#[0-9a-fA-F]{3,8}\b/.test(css), false);
  assert.equal(css.includes("/*"), false);
});

test("the phone startup sheet sits with the other phone sheets, before the off-key sheet", () => {
  const links = [...html.matchAll(/<link rel="stylesheet" id="(studio-phone[a-z-]*)"/g)].map(m => m[1]);
  assert.ok(links.includes("studio-phone-start"));
  assert.equal(links.at(-1), "studio-phone-off");
});

test("the phone startup screen wears the new look: the star, a slab of steps, and a full-width key", () => {
  assert.match(rule(" .startup"), /padding: calc\(env\(safe-area-inset-top\) \+ 24px\) 20px calc\(env\(safe-area-inset-bottom\) \+ 24px\);/);
  assert.match(rule(" .startup"), /radial-gradient\(460px 420px at 18% 6%, var\(--wall-lit\), transparent 72%\)/);
  assert.match(rule(" .startup-card > .startup-logo"), /display: none;/);
  assert.match(rule(" .startup-card > .start-star"), /--star-size: 76px;\n  display: block;/);
  assert.match(rule(" .startup-title"), /font: 700 25px\/1\.14 var\(--font-display\);/);
  assert.match(rule(" .reopen-text"), /font: 400 16px\/1\.45 var\(--font\);/);
  assert.match(rule(" .startup-steps"), /border-radius: var\(--radius-xl\);\n  background: [^;]*var\(--panel\);/);
  assert.match(rule(" .startup-step"), /height: 64px;/);
  assert.match(rule(" .startup-name"), /font: 600 16px\/1 var\(--font\);/);
  assert.match(rule(" .startup-skip"), /width: 100%;\n  height: 50px;[\s\S]*border-radius: var\(--radius-pill\);\n  background: var\(--k-plain\);/);
});

test("the phone apps never show the Updates step, because the store updates them", () => {
  assert.match(rule(".mobile-app #startup-update"), /display: none;/);
  assert.match(html, /id="startup-update"/);
});

test("each step shows its state with a mark, and the waiting mark holds still for people who ask", () => {
  assert.match(rule(' .startup-step[data-state="working"] .startup-mark'), /background: radial-gradient\(circle, var\(--yellow\) 4\.5px, transparent 5px\);[\s\S]*animation: pt-start-wait 1\.6s ease-in-out infinite;/);
  assert.match(rule(' .startup-step[data-state="done"] .startup-mark'), /color: var\(--green\);/);
  assert.match(rule(' .startup-step[data-state="failed"] .startup-mark'), /color: var\(--danger-text\);/);
  assert.match(rule(' #startup.offline #startup-connection[data-state="failed"] .startup-mark'), /background: var\(--accent-2\);\n  color: var\(--accent-2-ink\);/);
  assert.ok(css.includes(`@media (prefers-reduced-motion: reduce) {\n  ${SLIM} .startup-step[data-state="working"] .startup-mark { animation: none; }\n}`));
});

test("the key that opens Commy early sits inside the card of steps, and the card does not jump when it arrives", () => {
  assert.match(rule(" .startup-skip"), /position: relative;\n  align-self: start;\n  width: 100%;\n  height: 50px;\n  margin: 26px 0 0;/);
  assert.match(rule(" .startup-card:has(> .startup-skip.shown) > .startup-steps"), /padding-bottom: 80px;/);
  assert.match(rule(" .startup-card:has(> .startup-skip.shown) > .startup-skip"), /width: calc\(100% - 36px\); margin-top: -66px;/);
});
