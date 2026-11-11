import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-off.css");
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';

test("every off-key rule is for the phone layout of the new look, with tokens and no comments", () => {
  const selectors = css.split("}").map(block => block.split("{")[0]).flatMap(head => head.split(",")).map(s => s.trim()).filter(Boolean);
  assert.equal(selectors.length, 6);
  for (const selector of selectors) assert.ok(selector.startsWith(SLIM), selector);
  assert.equal(/#[0-9a-fA-F]{3,8}\b/.test(css), false);
  assert.equal(css.includes("/*"), false);
});

test("the off-key sheet loads after every other phone sheet", () => {
  const links = [...html.matchAll(/<link rel="stylesheet" id="(studio-phone[a-z-]*)"/g)].map(m => m[1]);
  assert.equal(links.at(-1), "studio-phone-off");
  assert.ok(links.includes("studio-phone-forum"));
});

test("a primary key that cannot be pressed yet wears the guide's off look, never a dimmed ultramarine", () => {
  const day = css.slice(0, css.indexOf("}"));
  for (const part of [".btn-primary:disabled", ".key.primary:disabled", ".key.primary.is-off"]) assert.ok(day.includes(`${SLIM} ${part}`), part);
  assert.match(day, /opacity: 1 !important;/);
  assert.match(day, /background: var\(--k-off\) !important;/);
  assert.match(day, /box-shadow: var\(--k-off-shadow\) !important;/);
  assert.match(day, /color: var\(--text-muted\) !important;/);
  const night = css.slice(css.indexOf(`${SLIM}:not([data-theme="light"]) .btn-primary:disabled`));
  assert.match(night, /background: var\(--glass-key-off\) !important;\n  box-shadow: inset 0 0 0 1px rgb\(var\(--lamp\) \/ \.1\) !important;/);
});
