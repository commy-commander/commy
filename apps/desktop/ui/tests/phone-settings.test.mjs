import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-settings.css");
const glue = read("scripts", "phone-settings.js");
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';

const rule = selector => {
  const at = css.indexOf(`${SLIM} ${selector} {`);
  assert.notEqual(at, -1, selector);
  return css.slice(at, css.indexOf("}", at));
};

test("every phone settings rule is for the phone layout of the new look, with tokens and no comments", () => {
  const selectors = css.split("}").map(block => block.split("{")[0]).flatMap(head => head.split(",")).map(s => s.trim()).filter(Boolean);
  assert.ok(selectors.length > 30);
  for (const selector of selectors) assert.ok(selector.startsWith(SLIM), selector);
  assert.equal(/#[0-9a-fA-F]{3,8}\b(?! 0 0)/.test(css.replace(/#000 calc/g, "").replace(/#000 0 0/g, "")), false);
  assert.equal(css.includes("/*"), false);
  assert.equal(/\/\/|\/\*/.test(glue.replace(/https?:\/\//g, "")), false);
});

test("the phone settings files sit with the other phone files", () => {
  assert.match(html, /id="studio-phone" href="styles\/phone-touch\.css" media="not all" \/>\n(\s*<link rel="stylesheet" id="studio-phone-[a-z-]+" href="styles\/phone-[a-z-]+\.css" media="not all" \/>\n)*\s*<link rel="stylesheet" id="studio-phone-settings" href="styles\/phone-settings\.css" media="not all" \/>/);
  assert.match(html, /<script src="scripts\/studio-phone\.js"><\/script>\n(\s*<script src="scripts\/phone-[a-z-]+\.js"><\/script>\n)*\s*<script src="scripts\/phone-settings\.js"><\/script>/);
});

test("settings menus are pushed screens: a back key, the title with its small line, no close key", () => {
  assert.match(rule(".settings-layer > .settings-phone-bar"), /height: 60px;.*padding: 0 14px 0 6px;/);
  assert.match(rule(".settings-layer .settings-phone-bar > .settings-phone-back"), /display: grid;.*width: 44px; height: 44px;/);
  assert.match(rule(".settings-layer .settings-phone-bar > .settings-phone-close"), /display: none;/);
  assert.match(rule(".settings-layer .settings-phone-title"), /font: 700 21px\/1 var\(--font-display\);/);
  assert.match(rule(".settings-layer .settings-phone-title > em"), /font-weight: 300; color: var\(--link\);/);
  assert.match(rule(".settings-layer .settings-phone-sub"), /margin-top: 5px;.*font: 400 12\.5px\/1 var\(--font\); color: var\(--text-normal\);/);
  assert.match(glue, /layer\.dataset\.phoneView === "page"\) return;\n\s*e\.preventDefault\(\);\n\s*e\.stopPropagation\(\);\n\s*layer\.querySelector\("\.settings-phone-close"\)\.click\(\);/);
  assert.match(glue, /if \(layer\.id === "server-settings"\) return page \? \$\("server-settings-title"\)\.textContent : layer\.getAttribute\("aria-label"\);/);
  assert.match(glue, /if \(layer\.id === "channel-settings" && !page\) return layer\.getAttribute\("aria-label"\);/);
});

test("the menu and the page sit on one slab that runs off the bottom", () => {
  const slab = css.slice(css.indexOf(`${SLIM} .settings-layer > .settings-nav,`), css.indexOf("}", css.indexOf(`${SLIM} .settings-layer > .settings-nav,`)));
  assert.match(slab, /left: 10px;\n  right: 10px;\n  top: calc\(env\(safe-area-inset-top\) \+ 66px\);\n  bottom: -40px;/);
  assert.match(slab, /border-radius: var\(--radius-xl\);/);
  assert.match(rule(".settings-layer .settings-nav > .settings-nav-inner"), /padding: 14px 12px calc\(var\(--hz-safe\) \+ 24px\);/);
});

test("menu rows: labels, 52 rows with the icon, a faint chevron, and red danger rows under a hairline", () => {
  assert.match(rule(".settings-layer .settings-nav-inner > .settings-nav-group"), /height: 34px;.*padding: 14px 12px 0;.*font: 600 11\.5px\/20px var\(--font\);.*text-transform: none; color: var\(--text-faint\);/);
  assert.match(rule(".settings-layer .settings-nav-inner > .settings-tab"), /gap: 13px;.*height: 52px;.*padding: 0 6px 0 12px;.*border-radius: 12px;.*font: 500 16px\/1 var\(--font\);/);
  assert.match(rule(".settings-layer .settings-nav-inner > .settings-tab:active"), /background: var\(--bg-hover\);/);
  assert.match(rule(".settings-layer .settings-nav-inner > .settings-tab > .i"), /width: 20px; height: 20px; color: var\(--text-muted\);/);
  assert.match(rule(".settings-layer .settings-nav-inner > .settings-tab > .pt-set-chev"), /margin: 0 8px 0 auto; color: var\(--text-faint\);/);
  assert.match(rule(".settings-layer .settings-nav-inner > .settings-tab.danger"), /color: var\(--danger-text\);/);
  assert.match(rule(".settings-layer .settings-nav-inner > .settings-nav-sep"), /height: 1px; margin: 8px 12px;.*background: var\(--line\);/);
  assert.match(rule(".settings-layer .settings-nav-inner .settings-search"), /height: 44px;.*border-radius: 999px;/);
  assert.match(glue, /const wants = slim && !tab\.classList\.contains\("danger"\);/);
  assert.match(rule("#channel-settings .settings-nav-inner > #channel-settings-title"), /display: none;/);
});
