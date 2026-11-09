import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { inAppSignIn, inAppRetry, signInStepView, readCaptchaMessage } = require("../scripts/signin-flow.js");

const browserOnly = { form: false, password: false, apple: false, google: false, twitch: false, passkey: false, buttons: false, captchaUrl: null };

test("the desktop app and older servers keep the browser sign-in", () => {
  assert.deepEqual(inAppSignIn({ password: true, apple: true, google: true, passkey: true, captcha_url: "https://commy.chat/x" }, false), browserOnly);
  assert.deepEqual(inAppSignIn(null, true), browserOnly);
  assert.equal(inAppSignIn({ password: false, apple: false }, true).form, false);
});

test("phones sign in inside the app, with Apple only where the app offers it", () => {
  const android = inAppSignIn({ password: true, apple: false, captcha_url: null }, true);
  assert.deepEqual(android, { ...browserOnly, form: true, password: true });
  const iphone = inAppSignIn({ password: true, apple: true, captcha_url: "https://commy.chat/api/auth/commy/captcha" }, true);
  assert.equal(iphone.apple, true);
  assert.equal(iphone.captchaUrl, "https://commy.chat/api/auth/commy/captcha");
});

test("Google, Twitch and passkeys show up when the server and the phone offer them", () => {
  const view = inAppSignIn({ password: true, apple: false, google: true, twitch: false, passkey: true, captcha_url: null }, true);
  assert.equal(view.google, true);
  assert.equal(view.twitch, false);
  assert.equal(view.passkey, true);
  assert.equal(view.buttons, true);
  const buttonsOnly = inAppSignIn({ password: false, twitch: true }, true);
  assert.equal(buttonsOnly.form, true, "the form shows even when only a social button is offered");
  assert.equal(buttonsOnly.password, false);
});

test("confirming age or the access code retries the sign-in that asked for it", () => {
  assert.deepEqual(inAppRetry({ kind: "sheet", provider: "google" }, { adultConfirmed: true }), {
    command: "account_sign_in_sheet",
    args: { provider: "google", adultConfirmed: true, accessCode: null },
  });
  assert.deepEqual(inAppRetry({ kind: "apple" }, { adultConfirmed: true, accessCode: "abc" }), {
    command: "account_sign_in_apple",
    args: { create: true, adultConfirmed: true, accessCode: "abc" },
  });
  assert.equal(inAppRetry({ kind: "sheet", provider: "github" }).command, "account_sign_in_apple");
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
