import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { typedCode, deleteConfirmView, deleteProof } = require("../scripts/account-delete.js");

test("an emailed code is asked for first, with the password as a fallback when there is one", () => {
  const view = deleteConfirmView({ confirm: "email", password: true, email: "me@example.test" });
  assert.equal(view.mode, "code");
  assert.equal(view.canUsePassword, true);
  assert.match(view.sentLine, /me@example\.test/);
  assert.equal(deleteConfirmView({ confirm: "email", password: false, email: "me@example.test" }).canUsePassword, false);
});

test("a server without email asks for the password, or relies on a fresh sign-in", () => {
  assert.equal(deleteConfirmView({ confirm: "password", password: true }).mode, "password");
  assert.equal(deleteConfirmView({ confirm: "fresh", password: false }).mode, "none");
  assert.equal(deleteConfirmView(null).mode, "none");
});

test("only a whole code or a typed password can confirm", () => {
  assert.deepEqual(deleteProof("code", "123 456", ""), { code: "123456", password: null });
  assert.deepEqual(deleteProof("code", "123-456", ""), { code: "123456", password: null });
  assert.equal(deleteProof("code", "12345", ""), null);
  assert.equal(deleteProof("code", "abcdef", ""), null);
  assert.deepEqual(deleteProof("password", "", "secret"), { code: null, password: "secret" });
  assert.equal(deleteProof("password", "123456", ""), null);
  assert.deepEqual(deleteProof("none", "", ""), { code: null, password: null });
  assert.equal(deleteProof("other", "123456", "x"), null);
  assert.equal(typedCode(" 12 34 56 "), "123456");
});
