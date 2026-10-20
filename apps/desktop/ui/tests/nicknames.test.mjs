import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { MAX_NICKNAME_CHARS, cleanNickname, nicknameProblem, nicknameAccountLine, nicknamePrompt } = require("../scripts/nicknames.js");

test("nicknames are trimmed, blank means cleared, and bad ones are told apart from cleared", () => {
  assert.equal(cleanNickname("  Sammy "), "Sammy");
  assert.equal(cleanNickname(""), null);
  assert.equal(cleanNickname("   "), null);
  assert.equal(cleanNickname(null), null);
  assert.equal(cleanNickname("n".repeat(MAX_NICKNAME_CHARS)), "n".repeat(MAX_NICKNAME_CHARS));
  assert.equal(cleanNickname("n".repeat(MAX_NICKNAME_CHARS + 1)), undefined);
  assert.equal(cleanNickname("Sam\u0007my"), undefined);
  assert.equal(cleanNickname("Sämmy 🦀"), "Sämmy 🦀");
});

test("only an unusable nickname has a problem to show", () => {
  assert.equal(nicknameProblem("Sammy"), null);
  assert.equal(nicknameProblem(""), null);
  assert.equal(nicknameProblem("n".repeat(40)), "Nicknames can be up to 32 characters");
});

test("the account name shows beside a nickname only when it adds something", () => {
  assert.equal(nicknameAccountLine({ name: "Sammy", username: "sam", nickname: "Sammy" }), "sam");
  assert.equal(nicknameAccountLine({ name: "sam", username: "sam", nickname: null }), null);
  assert.equal(nicknameAccountLine({ name: "sam", username: "sam", nickname: "sam" }), null);
  assert.equal(nicknameAccountLine({ id: "x", name: "sam" }), null);
  assert.equal(nicknameAccountLine(null), null);
});

test("the prompt says whose name changes and where", () => {
  const server = { name: "Rust Hangout" };
  assert.equal(nicknamePrompt({ id: "me", username: "you" }, server, "me"), "How your name appears in Rust Hangout");
  assert.equal(nicknamePrompt({ id: "m1", username: "ferris" }, server, "me"), "How ferris appears in Rust Hangout");
  assert.equal(nicknamePrompt({ id: "m1" }, null, "me"), "How this member appears");
});
