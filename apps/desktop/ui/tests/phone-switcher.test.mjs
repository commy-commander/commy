import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-switcher.css");
const glue = read("scripts", "phone-switcher.js");
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';
globalThis.serverShownName = require("../scripts/server-names.js").serverShownName;
const { SWITCHER_NEWS_FACES, newsServers, newsSummary, newsCount, newsWords, optionsWords } = require("../scripts/phone-switcher.js");

const rule = selector => {
  const at = css.indexOf(`${SLIM} ${selector} {`);
  assert.notEqual(at, -1, selector);
  return css.slice(at, css.indexOf("}", at));
};

const servers = [
  { id: "pp", name: "Pixel Pioneers" },
  { id: "no", name: "Night Owls" },
  { id: "kc", name: "Kart Club" },
  { id: "bn", name: "Book Nook" },
  { id: "sl", name: "Synth Lab" },
];
const counts = { pp: 6, no: 3, kc: 0, bn: 1, sl: 120 };
const unread = id => counts[id] || 0;

test("every switcher rule is for the phone app in the new look, with tokens and no comments", () => {
  const flat = css.replace(/@media \([a-z-]+: [a-z]+\) \{/g, "");
  const selectors = flat.split("}").map(block => block.split("{")[0]).flatMap(head => head.split(/,(?![^(]*\))/)).map(s => s.trim()).filter(Boolean);
  assert.ok(selectors.length > 20);
  for (const selector of selectors) assert.ok(selector.startsWith(SLIM), selector);
  assert.equal(/#[0-9a-fA-F]{3,8}\b/.test(css.replace(/#server-list\b/g, "")), false);
  assert.equal(css.includes("/*"), false);
  assert.equal(/\/\/|\/\*/.test(glue.replace(/https?:\/\//g, "")), false);
  assert.equal(/:hover/.test(css), false);
});

test("the switcher files load with the phone files, before the off look, which stays last", () => {
  assert.match(html, /<link rel="stylesheet" id="studio-phone-picker" href="styles\/phone-picker\.css" media="not all" \/>\n\s*<link rel="stylesheet" id="studio-phone-switcher" href="styles\/phone-switcher\.css" media="not all" \/>/);
  const links = [...html.matchAll(/id="(studio-phone[a-z-]*)"/g)].map(m => m[1]);
  assert.equal(links.at(-1), "studio-phone-off");
  assert.match(html, /<script src="scripts\/phone-picker\.js"><\/script>\n\s*<script src="scripts\/phone-switcher\.js"><\/script>/);
});

test("news leaves out the server you are in, muted servers and quiet ones", () => {
  const items = newsServers(servers, "pp", unread, id => id === "sl");
  assert.deepEqual(items.map(item => item.server.id), ["no", "bn"]);
  assert.deepEqual(newsServers(servers, "pp", () => 0, () => false), []);
});

test("the capsule shows two faces and the count of everything new", () => {
  assert.equal(SWITCHER_NEWS_FACES, 2);
  const summary = newsSummary(newsServers(servers, "pp", unread, () => false));
  assert.deepEqual(summary.faces.map(server => server.id), ["no", "bn"]);
  assert.equal(summary.total, 124);
  assert.equal(summary.label, "99+");
  assert.equal(newsSummary(newsServers(servers, "pp", unread, id => id === "sl")).label, "4");
});

test("the capsule names the servers with news for screen readers", () => {
  const items = newsServers(servers, "pp", unread, () => false);
  assert.equal(newsWords([]), "Your servers");
  assert.equal(newsWords(items.slice(0, 1)), "New in Night Owls");
  assert.equal(newsWords(items.slice(0, 2)), "New in Night Owls and Book Nook");
  assert.equal(newsWords(items), "New in Night Owls, Book Nook and 1 more");
});

test("the capsule says its number counts new messages", () => {
  assert.equal(newsCount({ label: "1" }), "1 new");
  assert.equal(newsCount({ label: "99+" }), "99+ new");
  assert.match(glue, /news\.title = newsWords\(items\);/);
});

test("server names drop emoji codes in the capsule and options key", () => {
  const items = [{ server: { id: "gi", name: "Gabagool Industries :pepodancels:" }, count: 1 }];
  assert.equal(newsWords(items), "New in Gabagool Industries");
  assert.equal(optionsWords({ name: ":pepodancels: Gabagool Industries" }), "Gabagool Industries options");
});

test("the row of servers gives way to the name on Home, and the sheet borrows the app's own server list", () => {
  assert.match(rule('.app.pt-switch[data-pane="list"] > .rail'), /display: none;/);
  assert.match(glue, /\$\("pt-servers-slot"\)\.appendChild\(list\);/);
  assert.match(glue, /home\.parent\.insertBefore\(\$\("server-list"\), /);
  assert.match(glue, /news\.hidden = !switcherOn\(\) \|\| !items\.length;/);
  assert.match(rule(".pt-servers-slot > #server-list"), /grid-template-columns: repeat\(4, minmax\(0, 1fr\)\);/);
});

test("the sheet keeps Add a server, Discover and the server's own options in reach", () => {
  assert.match(glue, /switcherKey\("plus", "Add a server", "add-server-btn"\), switcherKey\("compass", "Discover", "discover-btn"\)/);
  assert.match(glue, /options\.setAttribute\("aria-label", "Server options"\);/);
  assert.match(glue, /closeServerSwitcher\(\(\) => openServerMenu\(state\.currentServer, 0, 0\)\)/);
  assert.match(rule(".pt-servers-keys > .pt-servers-key"), /height: 52px;/);
  assert.match(rule(".pt-servers-head > .pt-servers-options"), /width: 44px; height: 44px;/);
  assert.match(rule(".sidebar-header > .pt-news:not([hidden])"), /height: 44px;/);
});

test("the sheet closes with back, a tap outside or a pull down", () => {
  assert.match(glue, /scrim\.onclick = \(\) => closeServerSwitcher\(\);/);
  assert.match(glue, /layerStep\("ptServers", true, phoneSwitcher\.step, history\.state\)/);
  assert.match(glue, /sheetSettles\(drag\.dy, dragSpeed\(drag\.samples, e\.timeStamp\)\.y, SWIPE\.sheetAt\)/);
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{\n  html\[data-look="studio"\]\[data-fit="slim"\] \.pt-servers > \.pt-servers-sheet \{ animation: none; transition: none; \}/);
});

test("the options key names whose options it opens", () => {
  assert.equal(optionsWords({ name: "Pixel Pioneers" }), "Pixel Pioneers options");
  assert.equal(optionsWords(null), "Server options");
  assert.match(glue, /\$\("pt-servers-options"\)\.setAttribute\("aria-label", optionsWords\(plainServer\(\)\)\);/);
});

test("a long press on a tile or on the server name opens that server's own menu", () => {
  assert.match(glue, /slot\.addEventListener\("contextmenu", e => \{\n    const tile = e\.target\.closest\("\.server-icon:not\(\.waiting\), \.folder-head"\);/);
  assert.match(glue, /closeServerSwitcher\(\(\) => tile\.dispatchEvent\(new MouseEvent\("contextmenu"/);
  assert.match(glue, /\$\("server-name-header"\)\.addEventListener\("contextmenu", e => \{\n    if \(!switcherOn\(\) \|\| e\.target\.closest\("button"\)\) return;/);
});
