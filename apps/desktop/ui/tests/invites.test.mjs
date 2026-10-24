import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { findInviteLink, looksLikeInviteLink, checkInviteName, inviteTimeLeft, inviteLabel, sortInvites } = require("../scripts/invites.js");

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const long = "q".repeat(43);
const short = "Zx9-_".repeat(4) + "ab";

test("random codes and custom names with their key are found in messages", () => {
  assert.equal(findInviteLink("join https://commy.chat/i/KQ7M3N4P now"), "https://commy.chat/i/KQ7M3N4P");
  assert.equal(findInviteLink(`see https://commy.chat/i/KQ7M3N4P#${long}!`), `https://commy.chat/i/KQ7M3N4P#${long}`);
  assert.equal(findInviteLink(`come to https://commy.chat/i/pixel-pioneers#${short} tonight`), `https://commy.chat/i/pixel-pioneers#${short}`);
  assert.equal(findInviteLink(`commy://i/commy.chat/team42#${short}`), `commy://i/commy.chat/team42#${short}`);
  assert.ok(looksLikeInviteLink(`  https://commy.chat/i/pixel-pioneers#${short} `));
});

test("ordinary links that only look a little like invites are left alone", () => {
  for (const text of [
    "https://example.org/i/pixel-pioneers",
    "https://example.org/i/KQ7M3N4O",
    `https://commy.chat/i/pixel-pioneers#${short}x`,
    `https://commy.chat/i/pixel--pioneers#${short}`,
    "https://example.org/images/cat.png",
  ]) {
    assert.equal(looksLikeInviteLink(text), false, text);
  }
});

test("custom names follow the same rules as the app's backend", () => {
  assert.deepEqual(checkInviteName("  Pixel-Pioneers "), { name: "pixel-pioneers", error: null });
  assert.deepEqual(checkInviteName(""), { name: "", error: null });
  for (const bad of ["ab", "-lead", "trail-", "two--dashes", "has space", "admin", "pixelpio", "a".repeat(33)]) {
    assert.ok(checkInviteName(bad).error, bad);
  }
  assert.equal(checkInviteName("pixel-pi").error, null);
  const rust = readFileSync(join(repo, "crates", "commy-net", "src", "invite.rs"), "utf8");
  const reserved = /const RESERVED_NAMES: &\[&str\] = &\[([\s\S]*?)\];/.exec(rust)[1].match(/"([^"]+)"/g).map(s => s.slice(1, -1));
  assert.ok(reserved.length > 10);
  for (const name of reserved) assert.match(checkInviteName(name).error || "", /reserved|random invite code/, name);
});

test("time left reads plainly and counts down", () => {
  const now = 1_000_000_000_000;
  assert.equal(inviteTimeLeft(null, now), "Never expires");
  assert.equal(inviteTimeLeft(now - 1, now), "Expired");
  assert.equal(inviteTimeLeft(now + 30e3, now), "Less than a minute left");
  assert.equal(inviteTimeLeft(now + 60e3, now), "1 minute left");
  assert.equal(inviteTimeLeft(now + 45 * 60e3, now), "45 minutes left");
  assert.equal(inviteTimeLeft(now + 90 * 60e3, now), "1h 30m left");
  assert.equal(inviteTimeLeft(now + 12 * 3600e3, now), "12 hours left");
  assert.equal(inviteTimeLeft(now + 26 * 3600e3, now), "1 day left");
  assert.equal(inviteTimeLeft(now + 7 * 86400e3, now), "7 days left");
});

test("invites list newest first by their link", () => {
  const a = { host: "commy.chat", code: "KQ7M3N4P", created_at: 1 };
  const b = { host: "commy.chat", code: "pixel-pioneers", created_at: 2 };
  assert.equal(inviteLabel(b), "commy.chat/i/pixel-pioneers");
  assert.deepEqual(sortInvites([a, b]), [b, a]);
  assert.deepEqual(sortInvites(null), []);
});
