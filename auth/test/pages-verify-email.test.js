import assert from "node:assert/strict";
import { test } from "node:test";
import { verifyEmailOutcome } from "../pages/src/lib/verify-email.js";

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
