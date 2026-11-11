import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { WEB_UNLOCK_DIALOG, WEB_UNLOCK_KEYS, CAM_FOCUS_KEY, webUnlockStarState, webDressPlan, signinGoBright } = require("../scripts/studio-web.js");

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio-web.css");
const app = read("../scripts/app.js");
const glue = read("../scripts/studio-web.js");
const bridge = read("../../../web/ui/web-bridge.js");
const R = 'html[data-look="studio"]:not([data-fit="slim"])';
const selectors = [...css.replace(/@keyframes[^{]+\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, "").replace(/@(media|container)[^{]+\{/g, "").matchAll(/(^|\})\s*([^{}@]+)\{/g)]
  .flatMap(match => match[2].split(/,\n/).map(part => part.trim()).filter(Boolean));
const rule = selector => {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};
const U = `${R} #modal-web-unlock.modal.dialog.dc`;

test("the browser's own parts load with the new look, roomy only, from the tokens", () => {
  const anchor = '  <link rel="stylesheet" id="studio-saved" href="styles/studio-saved.css" media="not all" />\n';
  assert.ok(html.includes(`${anchor}  <link rel="stylesheet" id="studio-web" href="styles/studio-web.css" media="not all" />\n`));
  assert.ok(html.includes('  <script src="scripts/studio-saved.js"></script>\n  <script src="scripts/studio-web.js"></script>\n'));
  assert.ok(selectors.length > 15);
  for (const selector of selectors) assert.ok(selector.startsWith(R), selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual(css.match(/#[0-9a-fA-F]{3,8}\b/g), null);
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
  assert.doesNotMatch(glue, /\/\/|\/\*/);
});

test("the glue starts once, after the floating parts", () => {
  const line = "  if (studioLook()) studioWeb();\n";
  assert.equal(app.split(line).length, 2);
  assert.ok(app.indexOf(line) > app.indexOf("  if (studioLook()) studioFloat();\n"));
});

test("opening your servers in the browser is the guide's sign-in dialog", () => {
  assert.equal(WEB_UNLOCK_DIALOG, "dialog slab dc");
  assert.deepEqual(WEB_UNLOCK_KEYS, { "web-unlock-ask": "key primary", "web-unlock-code-use": "key" });
  for (const id of ["modal-web-unlock", ...Object.keys(WEB_UNLOCK_KEYS), "web-unlock-wait", "web-unlock-ask-row", "web-unlock-code", "web-unlock-error", "web-unlock-detail"]) assert.ok(bridge.includes(`"${id}"`), id);
  assert.equal(webUnlockStarState(true, false), "waiting");
  assert.equal(webUnlockStarState(false, true), "unimpressed");
  assert.equal(webUnlockStarState(false, false), "idle");
  assert.ok(glue.includes('studioIconMarkup("circle-alert")'));
  assert.match(rule(`${U} > #web-unlock-ask-row > #web-unlock-ask.key`), /height: 46px;/);
  assert.match(rule(`${U} > #web-unlock-code-row > #web-unlock-code`), /height: 46px;/);
  assert.match(rule(`${U} > #web-unlock-code-row > #web-unlock-code`), /border-radius: var\(--radius-pill\);/);
  assert.match(rule(`${U} > #web-unlock-code-row > #web-unlock-code`), /box-shadow: var\(--slot\);/);
  assert.match(rule(`${U} > #web-unlock-code-row > #web-unlock-code-use.key`), /height: 44px;/);
  assert.match(rule(`${U} > #web-unlock-wait > .signin-spinner`), /width: 9px;/);
  assert.match(rule(`${U} > #web-unlock-wait > .signin-spinner`), /background: var\(--yellow\);/);
  assert.ok(css.includes("@keyframes web-wait { 50% { opacity: .35; } }"));
  assert.ok(css.includes(`@media (prefers-reduced-motion: reduce) {\n  ${U} > #web-unlock-wait > .signin-spinner { animation: none; }\n}`));
});

test("the sign-in dialog has one bright key: the browser key turns glass while the in-app form shows", () => {
  assert.equal(signinGoBright(true, false), true);
  assert.equal(signinGoBright(true, true), false);
  assert.equal(signinGoBright(false, false), false);
  assert.ok(app.includes('browser.classList.toggle("btn-secondary", view.form);'));
  assert.ok(glue.includes('new MutationObserver(studioWebSigninGo).observe(go, { attributes: true, attributeFilter: ["class"] });'));
  assert.ok(glue.includes('if (!go || !studioWebRoomy()) return;'));
});

test("the glue only undoes its own classes in the slim fit", () => {
  assert.deepEqual(webDressPlan(["modal"], "", "dialog slab dc", true), { add: ["dialog", "slab", "dc"], remove: [], record: ["dialog", "slab", "dc"] });
  assert.deepEqual(webDressPlan(["key", "sq"], "", "key sm sq", true), { add: ["sm"], remove: [], record: ["sm"] });
  assert.deepEqual(webDressPlan(["key", "sm", "sq"], "sm", "key sm sq", false), { add: [], remove: ["sm"], record: [] });
  assert.deepEqual(webDressPlan(["key"], "", "key", false), { add: [], remove: [], record: [] });
  assert.ok(glue.includes('for (const node of modal.querySelectorAll("[data-studio-web]")) node.remove();'));
});

test("the Hide Chat key over the camera strip is a glass key with its icon from the start", () => {
  assert.equal(CAM_FOCUS_KEY, "key sm sq");
  assert.ok(html.includes('id="cam-grid-focus"'));
  assert.ok(glue.includes('typeof paintFocusButtons === "function") paintFocusButtons();'));
  assert.match(rule(`${R} #cam-grid > #cam-grid-focus.key`), /position: absolute;/);
  assert.match(rule(`${R} #cam-grid > #cam-grid-focus.key`), /transform: none;/);
  assert.match(rule(`${R} .cam-grid:not(.vr-grid .cam-grid)`), /background: transparent;/);
});

test("a long notice wraps to three lines at most, and one line stays a capsule", () => {
  assert.match(rule(`${R} #toast.toast`), /min-height: 48px;/);
  assert.match(rule(`${R} #toast.toast`), /border-radius: 24px;/);
  assert.match(rule(`${R} #toast.toast > .toast-text`), /white-space: normal;/);
  assert.match(rule(`${R} #toast.toast > .toast-text`), /-webkit-line-clamp: 3;/);
});

test("a live voice row gives its name the room the You tag took", () => {
  assert.match(rule(`${R} .voice-user:has(> .voice-live)`), /container-type: inline-size;/);
  assert.ok(css.includes(`@container (max-width: 280px) {\n  ${R} .voice-user > .voice-live ~ .voice-you { display: none; }\n}`));
});

test("in the browser, Switch is hidden like the room's, and the sign-in words stay the browser's", () => {
  assert.match(html, /<button class="vp-pill desktop-only native-only" id="vp-change-source"/);
  assert.ok(app.includes('tile.querySelector(".vr-switch").classList.toggle("hidden", inBrowserApp());'));
  const render = /function renderInAppSignIn\(view\) \{([\s\S]*?)\n\}/.exec(app)[1];
  assert.ok(render.indexOf("if (inBrowserApp()) return showSignInCheck(view.captchaUrl);") < render.indexOf("browser.textContent"));
  assert.ok(app.includes('if (id === "signin-intro" && inBrowserApp() && !account.signed_in) continue;'));
});

test("the desktop keeps its words; the browser drops the claims it cannot keep", () => {
  assert.ok(html.includes("Your messages and calls still go straight between devices"));
  assert.ok(html.includes("Theme, language, chat text and the Commy icon."));
  assert.ok(app.includes('"Servers are where your friends gather: text channels, voice and screen sharing, all connected directly between members. Start one, or join a friend\'s with an invite."'));
  assert.ok(app.includes('account.server_from_env && !inBrowserApp() ? " (set by COMMY_AUTH_URL)" : ""'));
  assert.ok(html.includes('<span class="native-only"> To use a different account server, sign out and choose it on the sign-in screen.</span>'));
});
