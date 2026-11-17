import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  inAppSignIn,
  signInModeText,
  returnSignInDue,
  signInFormParts,
  showsOfficialBadge,
  looksLikeEmail,
  emailNextStep,
  passkeyFallsBackToPassword,
  inAppRetry,
  signInStepView,
  readCaptchaMessage,
} = require("../scripts/signin-flow.js");

const browserOnly = { form: false, password: false, apple: false, google: false, twitch: false, passkey: false, passkeyInBrowser: false, browserSheet: false, buttons: false, socialIcons: false, signUp: false, captchaUrl: null };

test("the web app and older servers keep the browser sign-in", () => {
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

test("on a computer, Google and Twitch finish in the browser and a passkey opens the browser sign-in", () => {
  const desktop = inAppSignIn({ password: true, google: true, twitch: true, passkey: false, passkey_in_browser: true, browser_sheet: true, sign_up: true }, true);
  assert.equal(desktop.form, true);
  assert.equal(desktop.browserSheet, true);
  assert.equal(desktop.passkeyInBrowser, true);
  assert.equal(desktop.socialIcons, false);
  assert.equal(emailNextStep(desktop, false), "password", "no quiet passkey try on a computer");
  assert.equal(signInFormParts(desktop, "password").passkeyLink, true);
  assert.equal(inAppSignIn({ password: true, browser_sheet: true }, true).browserSheet, false, "no browser wait without a social button");
  assert.equal(inAppSignIn({ password: false, google: true, passkey_in_browser: true }, true).passkeyInBrowser, false, "the passkey link sits under the password");
  assert.equal(inAppSignIn({ password: true, passkey: true, passkey_in_browser: true }, true).passkeyInBrowser, false, "an in-app passkey wins");
});

test("on iPhone, Apple stays a full button and Google and Twitch shrink to icons", () => {
  assert.equal(inAppSignIn({ password: true, apple: true, google: true, twitch: true }, true).socialIcons, true);
  assert.equal(inAppSignIn({ password: true, apple: false, google: true, twitch: true }, true).socialIcons, false);
  assert.equal(inAppSignIn({ password: true, apple: true }, true).socialIcons, false);
});

test("the email comes first and the password only shows after it", () => {
  const view = inAppSignIn({ password: true, passkey: true, captcha_url: "https://commy.chat/check" }, true);
  const signInOnly = { name: false, adult: false, inbox: false, socials: true };
  assert.deepEqual(signInFormParts(view, "email"), { email: true, password: false, ...signInOnly, submitLabel: "signin.continue", passkeyLink: false, captchaUrl: null });
  assert.deepEqual(signInFormParts(view, "password"), {
    email: true,
    password: true,
    ...signInOnly,
    submitLabel: "signin.submit",
    passkeyLink: true,
    captchaUrl: "https://commy.chat/check",
  });
  const noPasskey = inAppSignIn({ password: true }, true);
  assert.equal(signInFormParts(noPasskey, "password").passkeyLink, false);
  const noPassword = inAppSignIn({ password: false, apple: true }, true);
  assert.deepEqual(signInFormParts(noPassword, "password"), { email: false, password: false, ...signInOnly, submitLabel: "signin.continue", passkeyLink: false, captchaUrl: null });
});

test("email sign-up asks for the email, then the details, then waits for the inbox", () => {
  const view = inAppSignIn({ password: true, sign_up: true, captcha_url: "https://commy.chat/check" }, true);
  assert.equal(view.signUp, true);
  const start = signInFormParts(view, "email", "signup");
  assert.deepEqual([start.email, start.password, start.name, start.adult, start.socials, start.captchaUrl], [true, false, false, false, true, null]);
  assert.equal(start.submitLabel, "signin.continue");
  const details = signInFormParts(view, "details", "signup");
  assert.deepEqual([details.email, details.password, details.name, details.adult, details.socials], [true, true, true, true, false]);
  assert.equal(details.submitLabel, "signup.submit");
  assert.equal(details.captchaUrl, "https://commy.chat/check");
  assert.equal(details.passkeyLink, false);
  const inbox = signInFormParts(view, "inbox", "signup");
  assert.deepEqual([inbox.email, inbox.password, inbox.inbox, inbox.socials], [false, false, true, false]);
  assert.equal(inbox.captchaUrl, "https://commy.chat/check", "signing in after confirming needs a fresh safety check");
});

test("servers without email sign-up keep sending new people to the browser", () => {
  assert.equal(inAppSignIn({ password: true, sign_up: false }, true).signUp, false);
  assert.equal(inAppSignIn({ password: false, sign_up: true }, true).signUp, false);
  assert.equal(signInFormParts(inAppSignIn({ password: true }, true), "details", "signup").name, false);
});

test("coming back to the app after the email link signs in, but not over and over", () => {
  const waiting = { stage: "inbox", mode: "signup", triedAt: 100000 };
  assert.equal(returnSignInDue({ ...waiting, now: 100000 + 15000 }), true);
  assert.equal(returnSignInDue({ ...waiting, now: 100000 + 3000 }), false, "a quick app switch does not resend the email");
  assert.equal(returnSignInDue({ ...waiting, now: 100000 + 3000, fromLink: true }), true, "the Open Commy link always signs in");
  assert.equal(returnSignInDue({ stage: "details", mode: "signup", now: 999999, fromLink: true }), false);
  assert.equal(returnSignInDue({ stage: "inbox", mode: "signin", now: 999999, fromLink: true }), false);
});

test("each mode names itself and points at the other", () => {
  assert.deepEqual(signInModeText("signup"), { title: "signup.title", intro: "signup.intro", apple: "signup.apple", switchLink: "signup.have.account" });
  assert.equal(signInModeText("signin").switchLink, "signin.create");
  assert.equal(signInModeText("anything").title, "signin.title");
});

test("after the email, a saved passkey is tried before the password", () => {
  const withPasskey = inAppSignIn({ password: true, passkey: true }, true);
  assert.equal(emailNextStep(withPasskey, false), "passkey");
  assert.equal(emailNextStep(withPasskey, true), "password", "a password the phone already filled in skips the passkey");
  assert.equal(emailNextStep(inAppSignIn({ password: true }, true), false), "password");
});

test("a missing, closed or refused passkey falls back to the password", () => {
  assert.equal(passkeyFallsBackToPassword({ step: "canceled" }), true);
  assert.equal(passkeyFallsBackToPassword({ step: "failed" }), true);
  assert.equal(passkeyFallsBackToPassword({ step: "done" }), false);
  assert.equal(passkeyFallsBackToPassword({ step: "code" }), false);
  assert.equal(passkeyFallsBackToPassword(null), false, "a sign-in already running is left alone");
});

test("only the official Commy.chat server wears the verified check, once it proves itself", () => {
  const proven = { default_server_host: "commy.chat", server_verified: true };
  assert.equal(showsOfficialBadge("default", proven), true);
  assert.equal(showsOfficialBadge("default", { server_verified: true }), true);
  assert.equal(showsOfficialBadge("default", { ...proven, server_verified: false }), false, "the name alone is not enough");
  assert.equal(showsOfficialBadge("default", { default_server_host: "commy.chat" }), false, "no badge before the server proves itself");
  assert.equal(showsOfficialBadge("default", undefined), false);
  assert.equal(showsOfficialBadge("default", { ...proven, default_server_host: "staging.commy.chat" }), false, "test servers are not the official one");
  assert.equal(showsOfficialBadge("https://chat.example.org", proven), false);
  assert.equal(showsOfficialBadge("custom", proven), false);
});

test("only something shaped like an email moves on", () => {
  assert.equal(looksLikeEmail(" me@example.com "), true);
  assert.equal(looksLikeEmail("me@example"), false);
  assert.equal(looksLikeEmail("me example.com"), false);
  assert.equal(looksLikeEmail(""), false);
  assert.equal(looksLikeEmail(null), false);
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
  assert.deepEqual(inAppRetry({ kind: "email", email: "me@example.com", password: "long enough", name: "Nova" }, { accessCode: "abc", captcha: "tok" }), {
    command: "account_sign_up_email",
    args: { email: "me@example.com", password: "long enough", name: "Nova", adultConfirmed: true, captcha: "tok", accessCode: "abc" },
  });
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
