import assert from "node:assert/strict";
import { test } from "node:test";
import { APP_RETURN_LINK, APP_SIGN_UP_NEXT, verifyEmailOutcome } from "../pages/src/lib/verify-email.js";

test("a confirmed email continues where the sign-up started", () => {
  const outcome = verifyEmailOutcome("confirmed", "/web/");
  assert.equal(outcome.title, "Email confirmed");
  assert.deepEqual(outcome.action, { label: "Continue", href: "/web/" });
});

test("a confirmed email from a link that returns home continues to the account", () => {
  assert.equal(verifyEmailOutcome("confirmed", "/").action.href, "/account");
});

test("an expired link offers to sign in and get a new one", () => {
  const outcome = verifyEmailOutcome("expired", "/device?user_code=ABCD");
  assert.equal(outcome.title, "This link has expired");
  assert.match(outcome.body, /24 hours/);
  assert.deepEqual(outcome.action, { label: "Sign in", href: "/account?next=%2Fdevice%3Fuser_code%3DABCD" });
});

test("anything else reads as a link that did not work", () => {
  for (const result of ["invalid", undefined, "confirmed ", ["confirmed"]]) {
    const outcome = verifyEmailOutcome(result, "/account");
    assert.equal(outcome.result, "invalid");
    assert.equal(outcome.title, "This link did not work");
    assert.deepEqual(outcome.action, { label: "Sign in", href: "/account" });
  }
});

test("a confirmed email names the server the account lives on", () => {
  assert.match(verifyEmailOutcome("confirmed", "/account", "Pixel Home").body, /your Pixel Home account is ready/);
});

test("an account made in the phone app sends people straight back to the app", () => {
  const confirmed = verifyEmailOutcome("confirmed", APP_SIGN_UP_NEXT);
  assert.equal(confirmed.title, "Email confirmed");
  assert.match(confirmed.body, /Open Commy to finish signing in/);
  assert.deepEqual(confirmed.action, { label: "Open Commy", href: "commy://email-confirmed" });
  for (const result of ["expired", "invalid"]) {
    const outcome = verifyEmailOutcome(result, APP_SIGN_UP_NEXT);
    assert.equal(outcome.action.href, APP_RETURN_LINK);
    assert.match(outcome.body, /Send the email again/);
  }
  assert.equal(verifyEmailOutcome("confirmed", "/account?app=2").action.label, "Continue");
});
