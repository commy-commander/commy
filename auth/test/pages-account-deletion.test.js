import assert from "node:assert/strict";
import { test } from "node:test";
import { DELETED_RIGHT_AWAY, deletionProof, NOT_DELETED, startMode, typedCode } from "../pages/src/lib/account-deletion.js";

test("an emailed code comes first, then the password, then a fresh sign-in", () => {
  assert.equal(startMode({ confirm: "email", password: true, email: "a@example.test" }), "code");
  assert.equal(startMode({ confirm: "password", password: true, email: null }), "password");
  assert.equal(startMode({ confirm: "fresh", password: false, email: null }), "none");
  assert.equal(startMode(null), "none");
});

test("only a whole code or a typed password confirms", () => {
  assert.deepEqual(deletionProof("code", "123 456", ""), { code: "123456" });
  assert.equal(deletionProof("code", "12345", ""), null);
  assert.equal(deletionProof("code", "12a456", ""), null);
  assert.deepEqual(deletionProof("password", "", "secret"), { password: "secret" });
  assert.equal(deletionProof("password", "123456", ""), null);
  assert.deepEqual(deletionProof("none", "", ""), {});
  assert.equal(typedCode(" 12-34 56 "), "123456");
});

test("the page says what goes and what stays", () => {
  assert.ok(DELETED_RIGHT_AWAY.some((line) => line.includes("passkeys")));
  assert.ok(DELETED_RIGHT_AWAY.some((line) => line.includes("Discover")));
  assert.ok(NOT_DELETED.some((line) => line.includes("90 days")));
});
