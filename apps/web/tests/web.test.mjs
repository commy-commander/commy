import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";
import { contentSecurityPolicy, injectWebShell, relayOrigins } from "../shell.mjs";

const require = createRequire(import.meta.url);
const { WEB_DEFAULTS, webDefault, webSignInUrl, webUnlockText, webReplyLine } = require("../ui/web-commands.js");

const repo = join(import.meta.dirname, "..", "..", "..");
const read = (...path) => readFileSync(join(repo, ...path), "utf8");
const rustCommands = new Set([...read("apps", "web", "src", "session.rs").matchAll(/^\s+"([a-z_]+)" =>/gm)].map(m => m[1]));

test("every command the app calls while starting up has an answer in the browser", () => {
  const boot = /async function boot\(\) \{([\s\S]*?)\n\}/.exec(read("apps", "desktop", "ui", "scripts", "app.js"))[1];
  const loaders = [...boot.matchAll(/await (load[A-Za-z]+)\(\)/g)].map(m => m[1]);
  const app = read("apps", "desktop", "ui", "scripts", "app.js");
  const called = new Set(["get_voice_settings", "get_user", "list_servers", "get_server", "get_messages", "send_message"]);
  for (const loader of loaders) {
    const body = new RegExp(`function ${loader}\\(\\) \\{([\\s\\S]*?)\\n\\}`).exec(app)[1];
    for (const [, cmd] of body.matchAll(/invoke\("([a-z_]+)"/g)) called.add(cmd);
  }
  const missing = [...called].filter(cmd => !webDefault(cmd).known && !rustCommands.has(cmd));
  assert.deepEqual(missing, []);
});

test("the browser answers each command once, either from defaults or from the backend", () => {
  const both = Object.keys(WEB_DEFAULTS).filter(cmd => rustCommands.has(cmd));
  assert.deepEqual(both, []);
});

test("defaults are fresh copies, so the app cannot change them for the next caller", () => {
  const first = webDefault("list_friends").value;
  first.friends.push({ id: "x" });
  assert.deepEqual(webDefault("list_friends").value.friends, []);
  assert.equal(webDefault("create_server").known, false);
});

test("sign-in goes to the same site's account page", () => {
  assert.equal(webSignInUrl("https://commy.chat"), "https://commy.chat/account");
});

test("the unlock text prefers the backend's detail", () => {
  assert.equal(webUnlockText({ state: "locked", detail: "Try again" }), "Try again");
  assert.match(webUnlockText({ state: "no_list", detail: "" }), /computer/);
  assert.match(webUnlockText(null), /recovery code/);
});

test("replies cross into the backend as a status line and body", () => {
  assert.equal(webReplyLine(200, '{"a":1}'), '200\n{"a":1}');
});

test("the page may only talk to its own site and the relay, and loads nothing from elsewhere", () => {
  const csp = contentSecurityPolicy(relayOrigins("https://relay.commy.chat:8443"));
  assert.match(csp, /default-src 'self'/);
  assert.match(csp, /connect-src 'self' https:\/\/relay\.commy\.chat:8443 wss:\/\/relay\.commy\.chat:8443;/);
  assert.match(csp, /frame-src 'none'/);
  assert.doesNotMatch(csp, /\*/);
});

test("the bridge loads before the app and the policy lands in the head", () => {
  const html = injectWebShell(read("apps", "desktop", "ui", "index.html"), relayOrigins("https://relay.commy.chat:8443"));
  const bridge = html.indexOf("scripts/web-bridge.js");
  const commands = html.indexOf("scripts/web-commands.js");
  assert.ok(commands > 0 && commands < bridge && bridge < html.indexOf('src="scripts/app.js"'));
  assert.ok(html.indexOf("Content-Security-Policy") < html.indexOf("</head>"));
  assert.ok(html.includes('href="styles/web.css"'));
});

test("the bridge keeps its helpers private, so it cannot replace a function the app declares", () => {
  const source = read("apps", "web", "ui", "web-bridge.js");
  assert.match(source, /^\(\(\) => \{/m);
  assert.doesNotMatch(source, /^(?:async\s+)?function\s/m);
});

test("a local test relay is reached over plain http and ws", () => {
  assert.deepEqual(relayOrigins("http://localhost:3340"), ["http://localhost:3340", "ws://localhost:3340"]);
});
