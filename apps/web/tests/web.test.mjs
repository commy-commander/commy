import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";
import { contentSecurityPolicy, injectWebShell, relayOrigins, webRelayList } from "../shell.mjs";

const require = createRequire(import.meta.url);
const { WEB_DEFAULTS, webDefault, webFileSrc, webSignInUrl, webUnlockText, webReplyLine, webRelays } = require("../ui/web-commands.js");

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
  const first = webDefault("list_audio_devices").value;
  first.inputs.push({ id: "x" });
  assert.deepEqual(webDefault("list_audio_devices").value.inputs, []);
  assert.equal(webDefault("create_server").known, false);
});

test("sign-in goes to the same site's account page", () => {
  assert.equal(webSignInUrl("https://commy.chat"), "https://commy.chat/account");
});

test("sign-in brings the visitor back to the web app's own path", () => {
  assert.equal(webSignInUrl("https://commy.chat", "/web/"), "https://commy.chat/account?next=%2Fweb%2F");
});

test("pictures from GIF and emoji search load through this site's account server", () => {
  assert.equal(webFileSrc("", "commy-media"), "/api/auth/commy/media");
  assert.equal(webFileSrc("blob:x", "asset"), "blob:x");
});

test("the browser's sign-in text is not replaced by the desktop's translations", () => {
  assert.match(read("apps", "web", "ui", "web-bridge.js"), /removeAttribute\("data-i18n"\)/);
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
  const html = injectWebShell(read("apps", "desktop", "ui", "index.html"), { relays: ["https://relay.commy.chat:8443"] });
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

test("a page served under a path gets a base, so its relative scripts and styles still load", () => {
  const index = read("apps", "desktop", "ui", "index.html");
  const html = injectWebShell(index, { relays: ["https://relay.commy.chat:8443"], base: "/web/" });
  const baseAt = html.indexOf('<base href="/web/" />');
  assert.ok(baseAt > html.indexOf("<head>"));
  assert.ok(baseAt < html.indexOf('rel="icon"') && baseAt < html.indexOf('href="styles/styles.css"'));
  assert.doesNotMatch(injectWebShell(index, { relays: ["https://relay.commy.chat:8443"] }), /<base /);
});

test("the page allows every relay it may be pointed at and names the first as its relay", () => {
  const relays = webRelayList(" https://relay.commy.chat:8443/, https://relay-staging.commy.chat:8443 ", "https://fallback.example");
  assert.deepEqual(relays, ["https://relay.commy.chat:8443", "https://relay-staging.commy.chat:8443"]);
  const html = injectWebShell(read("apps", "desktop", "ui", "index.html"), { relays });
  assert.match(html, /connect-src 'self' https:\/\/relay\.commy\.chat:8443 wss:\/\/relay\.commy\.chat:8443 https:\/\/relay-staging\.commy\.chat:8443 wss:\/\/relay-staging\.commy\.chat:8443;/);
  assert.match(html, /<meta name="commy-relays" content="https:\/\/relay\.commy\.chat:8443" \/>/);
  assert.deepEqual(webRelayList("", "https://fallback.example"), ["https://fallback.example"]);
});

test("the backend gets the relay the page names, or its built-in one when the page names none", () => {
  const page = content => ({ querySelector: () => (content === null ? null : { getAttribute: () => content }) });
  assert.equal(webRelays(page("https://relay-staging.commy.chat:8443")), "https://relay-staging.commy.chat:8443");
  assert.equal(webRelays(page(" ")), undefined);
  assert.equal(webRelays(page(null)), undefined);
});

test("every image the bridge shows ships with the app", () => {
  const images = [...read("apps", "web", "ui", "web-bridge.js").matchAll(/src="(images\/[^"]+)"/g)].map(m => m[1]);
  assert.ok(images.length > 0);
  for (const image of images) assert.ok(existsSync(join(repo, "apps", "desktop", "ui", image)), image);
});
