import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { PRIVACY_NOTICE, privacyNoticeTitle, agreedToCurrentPolicy, privacyNoticeDue, finishPrivacyNotice } = require("../scripts/privacy-notice.js");

const memoryStorage = () => {
  const items = new Map();
  return { getItem: k => (items.has(k) ? items.get(k) : null), setItem: (k, v) => items.set(k, String(v)), removeItem: k => items.delete(k) };
};

const NOW = Date.parse("2026-11-20T12:00:00Z");
const longAgo = { id: "u1", created_at: "2026-01-02T03:04:05.000Z" };

test("the notice shows once per update and stays dismissed", () => {
  const storage = memoryStorage();
  assert.equal(privacyNoticeDue(storage, longAgo, NOW), true);
  finishPrivacyNotice(storage);
  assert.equal(privacyNoticeDue(storage, longAgo, NOW), false);
  assert.equal(privacyNoticeDue(storage, longAgo, NOW, "a-later-update"), true);
});

test("someone who just signed up already agreed, so the notice is marked seen", () => {
  const storage = memoryStorage();
  assert.equal(privacyNoticeDue(storage, { id: "u2", created_at: "2026-11-20T11:59:00Z" }, NOW), false);
  assert.equal(privacyNoticeDue(storage, longAgo, NOW), false);
});

test("an account made on or after the update date agreed to it", () => {
  assert.equal(agreedToCurrentPolicy({ created_at: "2026-11-18T09:00:00Z" }, NOW, "2026-11-18"), true);
  assert.equal(agreedToCurrentPolicy({ created_at: "2026-11-17T09:00:00Z" }, NOW, "2026-11-18"), false);
  assert.equal(agreedToCurrentPolicy({ created_at: "2026-11-17T09:00:00Z" }, NOW, ""), false);
});

test("an account without a sign-up time still sees the notice", () => {
  assert.equal(privacyNoticeDue(memoryStorage(), { id: "u3" }, NOW), true);
  assert.equal(privacyNoticeDue(memoryStorage(), undefined, NOW), true);
});

test("a storage that throws never shows the notice", () => {
  const broken = { getItem() { throw new Error("blocked"); }, setItem() { throw new Error("blocked"); } };
  assert.equal(privacyNoticeDue(broken, longAgo, NOW), false);
  assert.doesNotThrow(() => finishPrivacyNotice(broken));
});

test("the title names the release date once it is set", () => {
  assert.equal(privacyNoticeTitle("2026-12-01", "en-US"), "We updated our Privacy Policy on December 1, 2026.");
  assert.equal(privacyNoticeTitle("", "en-US"), "We updated our Privacy Policy.");
  assert.equal(privacyNoticeTitle("not a date", "en-US"), "We updated our Privacy Policy.");
});

test("the notice links to the privacy page and keeps no tracking", () => {
  assert.equal(PRIVACY_NOTICE.url, "https://commy.chat/privacy");
  const html = readFileSync(join(import.meta.dirname, "..", "index.html"), "utf8");
  const modal = html.slice(html.indexOf('id="modal-privacy-notice"'), html.indexOf('id="modal-stream-deck"'));
  assert.ok(modal.includes("Read the Privacy Policy") && modal.includes('id="privacy-notice-done"'));
  assert.equal((modal.match(/<li>/g) || []).length, 6);
  assert.ok(html.includes('<script src="scripts/privacy-notice.js"></script>'));
});
