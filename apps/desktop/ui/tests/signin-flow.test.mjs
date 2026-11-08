import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { inAppSignIn, signInStepView, readCaptchaMessage } = require("../scripts/signin-flow.js");

test("the desktop app and older servers keep the browser sign-in", () => {
  assert.deepEqual(inAppSignIn({ password: true, apple: true, captcha_url: "https://commy.chat/x" }, false), { form: false, password: false, apple: false, captchaUrl: null });
  assert.deepEqual(inAppSignIn(null, true), { form: false, password: false, apple: false, captchaUrl: null });
  assert.equal(inAppSignIn({ password: false, apple: false }, true).form, false);
});

test("phones sign in inside the app, with Apple only where the app offers it", () => {
  const android = inAppSignIn({ password: true, apple: false, captcha_url: null }, true);
  assert.deepEqual(android, { form: true, password: true, apple: false, captchaUrl: null });
  const iphone = inAppSignIn({ password: true, apple: true, captcha_url: "https://commy.chat/api/auth/commy/captcha" }, true);
  assert.equal(iphone.apple, true);
  assert.equal(iphone.captchaUrl, "https://commy.chat/api/auth/commy/captcha");
});

test("each server answer opens its step", () => {
  assert.equal(signInStepView("code"), "code");
  assert.equal(signInStepView("new_account"), "new");
  assert.equal(signInStepView("confirm_adult"), "new");
  assert.equal(signInStepView("access_code"), "access");
  assert.equal(signInStepView("anything else"), "form");
});

test("only the account server's check page can hand over a check token", () => {
  const url = "https://commy.chat/api/auth/commy/captcha";
  const from = (origin, data) => readCaptchaMessage({ origin, data }, url);
  assert.deepEqual(from("https://commy.chat", { type: "commy-captcha", token: "t1" }), { token: "t1" });
  assert.deepEqual(from("https://commy.chat", { type: "commy-captcha", expired: true }), { expired: true });
  assert.deepEqual(from("https://commy.chat", { type: "commy-captcha", failed: true }), { failed: true });
  assert.equal(from("https://evil.example", { type: "commy-captcha", token: "t1" }), null);
  assert.equal(from("https://commy.chat", { type: "other", token: "t1" }), null);
  assert.equal(from("https://commy.chat", "commy-captcha"), null);
  assert.equal(readCaptchaMessage({ origin: "https://commy.chat", data: { type: "commy-captcha", token: "t" } }, null), null);
});
