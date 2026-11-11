import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-plus.css");
const glue = read("scripts", "phone-plus.js");
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';

const rule = selector => {
  const at = css.indexOf(`${SLIM} ${selector} {`);
  assert.notEqual(at, -1, selector);
  return css.slice(at, css.indexOf("}", at));
};

test("every plus rule is for the phone app in the new look, with tokens and no comments", () => {
  const flat = css.replace(/@media \(prefers-reduced-motion: reduce\) \{/, "");
  const selectors = flat.split("}").map(block => block.split("{")[0]).flatMap(head => head.split(/,(?![^(]*\))/)).map(s => s.trim()).filter(Boolean);
  assert.ok(selectors.length > 20);
  for (const selector of selectors) assert.ok(selector.startsWith(SLIM), selector);
  assert.equal(/#[0-9a-fA-F]{3,8}\b/.test(css), false);
  assert.equal(css.includes("/*"), false);
  assert.equal(/\/\/|\/\*/.test(glue.replace(/https?:\/\//g, "")), false);
});

test("the plus file sits with the other phone files and the off look stays last", () => {
  assert.match(html, /id="studio-phone" href="styles\/phone-touch\.css" media="not all" \/>\n(\s*<link rel="stylesheet" id="studio-phone-[a-z-]+" href="styles\/phone-[a-z-]+\.css" media="not all" \/>\n)*\s*<link rel="stylesheet" id="studio-phone-plus" href="styles\/phone-plus\.css" media="not all" \/>/);
  const links = [...html.matchAll(/id="(studio-phone[a-z-]*)"/g)].map(m => m[1]);
  assert.equal(links.at(-1), "studio-phone-off");
  assert.ok(links.includes("studio-phone-plus"));
  assert.match(html, /<script src="scripts\/studio-phone\.js"><\/script>\n(\s*<script src="scripts\/phone-[a-z-]+\.js"><\/script>\n)*\s*<script src="scripts\/phone-plus\.js"><\/script>/);
  assert.equal(read("scripts", "studio-phone.js").includes("BAR_EXTRAS"), false);
});

test("pressing plus folds the field away and five equal keys rise on the line", () => {
  assert.match(glue, /\{ id: "file", label: "File", icon: "file", press: "cm-file", files: true \}/);
  assert.match(glue, /\{ id: "image", label: "Image", icon: "image", press: "cm-photo", files: true \}/);
  assert.match(glue, /\{ id: "emoji", label: "Emoji", icon: "smile", press: "emoji-btn" \}/);
  assert.match(glue, /\{ id: "poll", label: "Poll", icon: "chart-bar-big", press: "cm-poll", poll: true \}/);
  assert.match(glue, /\{ id: "voice", label: "Voice", icon: "mic", press: "cm-voice", voice: true, title: "Record a Voice Message" \}/);
  assert.match(glue, /\(item\.voice && !voice\)/);
  assert.match(rule(".main > .composer.pt-plus-open > textarea"), /scale: \.05 1; opacity: 0; pointer-events: none;/);
  assert.match(rule(".main > .composer.pt-plus-open .composer-send"), /translate: 0 22px; opacity: 0;/);
  assert.match(rule(".main > .composer.pt-plus-open .composer-attach::before"), /rotate: 45deg;/);
  assert.match(rule(".main > .composer > .pt-plus"), /left: 56px;\n  right: -6px;\n  bottom: -51px;[\s\S]*grid-auto-columns: 1fr;\n  height: 92px;/);
  assert.match(rule(".pt-plus .pt-plus-k"), /grid-template-rows: 48px 44px;[\s\S]*font: 600 12px\/1 var\(--font\);[\s\S]*color: var\(--tab-label\);[\s\S]*translate: 0 12px;/);
  assert.match(rule(".pt-plus .ic"), /width: 28px;\n  height: 26px;\n  margin-bottom: 8px;\n  color: var\(--header-primary\);/);
  assert.match(rule(".pt-plus .ic .i"), /width: 25px; height: 25px;/);
  assert.match(rule(".main > .composer.pt-plus-open .pt-plus-k"), /transition: translate 300ms var\(--ease-spring\), opacity 140ms var\(--ease-out\)/);
  assert.match(rule(".main > .composer.pt-plus-open .pt-plus-k:nth-child(4)"), /transition-delay: 160ms, 160ms, 0s;/);
  assert.match(rule(".main > .composer.pt-plus-open .pt-plus-k:nth-child(5)"), /transition-delay: 200ms, 200ms, 0s;/);
});

test("the plus row is hidden until the phone layout shows it, and the keys press the app's own actions", () => {
  assert.match(glue, /keys\.hidden = true;/);
  assert.match(glue, /setTimeout\(\(\) => \$\(item\.press\)\.click\(\), 0\);/);
  assert.match(glue, /\$\("attach-btn"\)\.setAttribute\("aria-label", open \? "Close" : "Attach"\);/);
  assert.match(glue, /\$\("message-input"\)\.blur\(\);/);
  assert.match(glue, /if \(!studioSlim\(\) \|\| !e\.target\.closest \|\| !e\.target\.closest\("#attach-btn"\)\) return;/);
  assert.match(rule(".pt-plus .pt-plus-k[hidden]"), /display: none;/);
});

test("Aa and GIF are small round muted keys at the right end of the field", () => {
  assert.match(glue, /\{ press: "text-style-btn", icon: "a-large-small" \},\n  \{ press: "gif-btn", icon: "gif" \}/);
  assert.match(rule(".main > .composer :is(#text-style-btn, #gif-btn)"), /width: 28px;[\s\S]*height: 28px;[\s\S]*border-radius: 50%;[\s\S]*color: var\(--text-muted\);/);
  assert.match(rule(".main > .composer :is(#text-style-btn, #gif-btn) > .pt-fk .i"), /width: 20px; height: 20px;/);
  assert.match(rule(".main > .composer #gif-btn.hidden"), /display: none;/);
  assert.match(css, /:not\(\[data-theme="light"\]\) \.main > \.composer #text-style-btn\[aria-expanded="true"\] \{\n  background: var\(--glass-hi\), var\(--glass-key-on\);/);
  assert.match(css, /\[data-theme="light"\] \.main > \.composer #text-style-btn\[aria-expanded="true"\] \{\n  background: linear-gradient\(180deg, rgb\(var\(--lamp\) \/ \.08\), rgb\(var\(--lamp\) \/ 0\) 70%\), var\(--bg-active\);\n  box-shadow: var\(--pillow-sm\);/);
});

test("the text style popover is a small glass popover above the field, not a sheet", () => {
  assert.match(rule(".text-style-pop:not(.hidden)"), /left: auto !important;[\s\S]*bottom: var\(--pt-ts-bottom[\s\S]*width: 300px;[\s\S]*border-radius: var\(--radius-lg\);[\s\S]*var\(--panel-thin\);/);
  assert.equal(read("scripts", "studio-phone.js").includes(".text-style-pop"), false);
});

test("the recording bar takes the field's place with Cancel as glass and Send as primary", () => {
  assert.match(rule(".main > .composer > .voice-rec"), /position: absolute;\n  inset: 1px 0;[\s\S]*border-radius: 24px;\n  background: var\(--bar-field\);/);
  assert.match(rule(".voice-rec :is(#voice-rec-cancel, #voice-rec-send)"), /height: 32px;[\s\S]*background: var\(--k-plain\);/);
  assert.match(rule(".voice-rec #voice-rec-send"), /--k-ink: var\(--accent-ink\);\n  background: var\(--pillow-top\), var\(--accent\);/);
});
