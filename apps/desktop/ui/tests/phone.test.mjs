import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";

const require = createRequire(import.meta.url);
const { isPhoneWidth, phonePaneAfter, phonePaneGoesDeeper, backKeepsGoing, PHONE_BACK_STEPS, rowOpensConversation } = require("../scripts/phone.js");

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

test("going back keeps stepping until the screen the person asked for shows", () => {
  assert.equal(backKeepsGoing("list", "chat", { pane: "chat", ptSheet: "pt-chat-menu" }, 0), true);
  assert.equal(backKeepsGoing("list", "chat", { pane: "chat" }, 1), true);
  assert.equal(backKeepsGoing("list", "list", {}, 2), false);
  assert.equal(backKeepsGoing("chat", "people", { pane: "people" }, 0), true);
  assert.equal(backKeepsGoing("chat", "chat", { pane: "chat" }, 0), false);
  assert.equal(backKeepsGoing("chat", "list", {}, 0), false);
});

test("going back stops at the first step, with nothing asked for, and before it could run away", () => {
  assert.equal(backKeepsGoing(null, "chat", { pane: "chat" }, 0), false);
  assert.equal(backKeepsGoing("list", "chat", {}, 0), false);
  assert.equal(backKeepsGoing("list", "chat", null, 0), false);
  assert.equal(backKeepsGoing("list", "chat", { pane: "chat" }, PHONE_BACK_STEPS), false);
});

test("the back step on the phone names its goal and the history handler walks to it", () => {
  const app = readFileSync(new URL("../scripts/app.js", import.meta.url), "utf8");
  assert.match(app, /phoneBack = \{ goal: to, steps: 0 \};\n\s+history\.back\(\);/);
  assert.match(app, /if \(!back \|\| !backKeepsGoing\(back\.goal, pane, entry, back\.steps\)\) return;\n\s+phoneBack = \{ goal: back\.goal, steps: back\.steps \+ 1 \};\n\s+history\.back\(\);/);
});

test("a panel opened from the chat menu takes the menu's place in the history", () => {
  const app = readFileSync(new URL("../scripts/app.js", import.meta.url), "utf8");
  assert.match(app, /if \(history\.state && history\.state\.ptSheet\) history\.replaceState\(entry, ""\);\n\s+else history\.pushState\(entry, ""\);/);
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

test("a tap that only stops a scroll does not open message actions", () => {
  const { tapEndsScroll } = require("../scripts/phone.js");
  assert.equal(tapEndsScroll(1000, 1100), true);
  assert.equal(tapEndsScroll(1000, 2000), false);
});

test("header buttons pressed from the people pane go back to the chat, except the people button", () => {
  const { headerTapLeavesPeople } = require("../scripts/phone.js");
  assert.equal(headerTapLeavesPeople("people", "pins-btn"), true);
  assert.equal(headerTapLeavesPeople("people", "members-btn"), false);
  assert.equal(headerTapLeavesPeople("chat", "pins-btn"), false);
});

test("the phone app viewport cannot zoom", () => {
  const { PHONE_APP_VIEWPORT } = require("../scripts/phone.js");
  assert.match(PHONE_APP_VIEWPORT, /maximum-scale=1/);
  assert.match(PHONE_APP_VIEWPORT, /user-scalable=no/);
  assert.match(PHONE_APP_VIEWPORT, /viewport-fit=cover/);
});

test("wide portrait phone screens such as the iPhone Duo inner display get the two pane layout", () => {
  const { phoneAppViewport, PHONE_APP_VIEWPORT, TWO_PANE_VIEWPORT, TWO_PANE_LAYOUT_WIDTH_PX, isPhoneWidth } = require("../scripts/phone.js");
  assert.equal(phoneAppViewport(669, true), TWO_PANE_VIEWPORT);
  assert.equal(isPhoneWidth(TWO_PANE_LAYOUT_WIDTH_PX), false);
  assert.equal(phoneAppViewport(466, true), PHONE_APP_VIEWPORT);
  assert.equal(phoneAppViewport(393, true), PHONE_APP_VIEWPORT);
  assert.equal(phoneAppViewport(601, true), PHONE_APP_VIEWPORT);
  assert.equal(phoneAppViewport(951, false), PHONE_APP_VIEWPORT);
  assert.match(TWO_PANE_VIEWPORT, /user-scalable=no/);
});

