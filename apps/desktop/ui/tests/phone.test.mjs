import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { isPhoneWidth, phonePaneAfter, phonePaneGoesDeeper, rowOpensConversation } = require("../scripts/phone.js");

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
