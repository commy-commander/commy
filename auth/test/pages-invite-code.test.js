import assert from "node:assert/strict";
import { test } from "node:test";
import { cleanInviteCode } from "../pages/src/lib/user-code.js";

test("random codes read in capitals and custom names in lower case", () => {
  assert.equal(cleanInviteCode("kq7m3n4p"), "KQ7M3N4P");
  assert.equal(cleanInviteCode("Pixel-Pioneers"), "pixel-pioneers");
  assert.equal(cleanInviteCode("team42"), "team42");
  assert.equal(cleanInviteCode("pixel%2Dpioneers"), "pixel-pioneers");
});

test("anything else reads as no code", () => {
  for (const bad of ["", "ab", "-lead", "two--dashes", "has space", "a".repeat(33), "%E0%A4%A", "../etc", "pixel_pioneers"]) {
    assert.equal(cleanInviteCode(bad), "", bad);
  }
});
