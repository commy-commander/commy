import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { isPhoneWidth, phonePaneAfter, phonePaneGoesDeeper, stepToChat, rowOpensConversation } = require("../scripts/phone.js");

test("phone widths stay well below the smallest desktop window", () => {
  assert.equal(isPhoneWidth(390), true);
  assert.equal(isPhoneWidth(700), true);
  assert.equal(isPhoneWidth(940), false);
});

test("opening a conversation shows the chat, and back walks out one pane at a time", () => {
  assert.equal(phonePaneAfter("list", "open"), "chat");
  assert.equal(phonePaneAfter("chat", "people"), "people");
  assert.equal(phonePaneAfter("people", "back"), "chat");
  assert.equal(phonePaneAfter("chat", "back"), "list");
  assert.equal(phonePaneAfter("list", "back"), "list");
});

test("the people button toggles between the people pane and the chat", () => {
  assert.equal(phonePaneAfter("people", "people"), "chat");
});

test("only moves deeper add a step that the system back gesture can undo", () => {
  assert.equal(phonePaneGoesDeeper("list", "chat"), true);
  assert.equal(phonePaneGoesDeeper("chat", "people"), true);
  assert.equal(phonePaneGoesDeeper("people", "chat"), false);
  assert.equal(phonePaneGoesDeeper("chat", "list"), false);
});

test("a stream or chat started from any pane lands on the chat pane, where the stage is", () => {
  assert.equal(stepToChat("list"), "open");
  assert.equal(stepToChat("people"), "back");
  assert.equal(stepToChat("chat"), null);
  for (const pane of ["list", "people"]) assert.equal(phonePaneAfter(pane, stepToChat(pane)), "chat");
});

test("the voice panel offers a way back to a stream being watched, shown only on phone widths", () => {
  const ui = join(import.meta.dirname, "..");
  const html = readFileSync(join(ui, "index.html"), "utf8");
  const css = readFileSync(join(ui, "styles", "styles.css"), "utf8");
  const app = readFileSync(join(ui, "scripts", "app.js"), "utf8");
  assert.match(html, /<div class="vp-watch hidden" id="vp-watch">[\s\S]*?id="vp-show-watch"[^>]*>Show Stream<\/button>[\s\S]*?id="vp-stop-watch"/);
  assert.match(css, /@media \(min-width: 701px\) \{ \.vp-show-watch \{ display: none; \} \}/);
  assert.match(app, /\$\("vp-show-watch"\)\.onclick = showWatchedStream;/);
  assert.match(app, /openStage\("watch", `\$\{name\}'s screen`\);\s*showWatchedStream\(\);/);
});

test("tapping a channel or a conversation opens it, but links, requests and row buttons do not", () => {
  assert.equal(rowOpensConversation(["channel"], false), true);
  assert.equal(rowOpensConversation(["channel", "dm-thread"], false), true);
  assert.equal(rowOpensConversation(["channel", "voice-channel"], false), true);
  assert.equal(rowOpensConversation(["channel", "sidebar-link"], false), false);
  assert.equal(rowOpensConversation(["channel", "dm-thread", "friend-request"], false), false);
  assert.equal(rowOpensConversation(["channel"], true), false);
  assert.equal(rowOpensConversation(["channel-group"], false), false);
});
