import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  DIRECTORY_CATEGORIES, DIRECTORY_BUCKETS, DIRECTORY_DESCRIPTION_MAX, directoryCategoryLabel, memberBucket, memberBucketLabel,
  normalizeDirectorySearch, directoryExcerpt, directoryDescriptionCounter, discoverySettingsState,
} = require("../scripts/directory.js");

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const read = (...path) => readFileSync(join(repo, ...path), "utf8");

test("the category list matches the account server and the desktop backend", () => {
  const ids = DIRECTORY_CATEGORIES.map(c => c.id);
  const auth = /export const CATEGORIES = \[([^\]]+)\]/.exec(read("auth", "src", "directory.js"))[1];
  assert.deepEqual([...auth.matchAll(/"([a-z]+)"/g)].map(m => m[1]), ids);
  const rust = /pub const CATEGORIES: &\[&str\] = &\[([^\]]+)\]/.exec(read("apps", "desktop", "src-tauri", "src", "directory.rs"))[1];
  assert.deepEqual([...rust.matchAll(/"([a-z]+)"/g)].map(m => m[1]), ids);
  assert.equal(new Set(ids).size, ids.length);
  assert.equal(directoryCategoryLabel("tech"), "Tech");
  assert.equal(directoryCategoryLabel("nope"), "Other");
});

test("member buckets round down to the account server's fixed sizes", () => {
  const auth = /export const MEMBER_BUCKETS = \[([^\]]+)\]/.exec(read("auth", "src", "directory.js"))[1];
  assert.deepEqual(auth.split(",").map(s => Number(s.trim())), DIRECTORY_BUCKETS);
  const rust = /pub const MEMBER_BUCKETS: &\[u32\] = &\[([^\]]+)\]/.exec(read("apps", "desktop", "src-tauri", "src", "directory.rs"))[1];
  assert.deepEqual(rust.split(",").map(s => Number(s.trim())), DIRECTORY_BUCKETS);
  assert.equal(memberBucket(37), 25);
  assert.equal(memberBucket(3), 10);
  assert.equal(memberBucket(0), 10);
  assert.equal(memberBucket(250), 250);
  assert.equal(memberBucket(999), 500);
  assert.equal(memberBucket(4000), 1000);
  assert.equal(memberBucket("abc"), 10);
  assert.equal(memberBucketLabel(25), "25+ members");
  assert.equal(memberBucketLabel(1000), "1,000+ members");
});

test("search text is flattened, lowercased and bounded", () => {
  assert.equal(normalizeDirectorySearch("  Rust   HELP "), "rust help");
  assert.equal(normalizeDirectorySearch(null), "");
  assert.equal(normalizeDirectorySearch("x".repeat(200)).length, 80);
});

test("card excerpts cut at a word and never end mid punctuation", () => {
  assert.equal(directoryExcerpt(""), "No description yet.");
  assert.equal(directoryExcerpt("Short and sweet."), "Short and sweet.");
  const long = "word ".repeat(60).trim();
  const excerpt = directoryExcerpt(long, 50);
  assert.ok(excerpt.endsWith("…"));
  assert.ok(excerpt.length <= 51);
  assert.doesNotMatch(excerpt, /\s…$/);
  assert.equal(directoryExcerpt("one, two, three, four", 10), "one, two…");
  assert.equal(directoryExcerpt("a".repeat(80), 20), "a".repeat(20) + "…", "a single giant word is simply cut");
});

test("the description counter tracks the account server's limit", () => {
  const auth = /export const DESCRIPTION_MAX = (\d+)/.exec(read("auth", "src", "directory.js"))[1];
  assert.equal(Number(auth), DIRECTORY_DESCRIPTION_MAX);
  assert.deepEqual(directoryDescriptionCounter("abc"), { text: "3 / 300", over: false });
  assert.deepEqual(directoryDescriptionCounter("x".repeat(301)), { text: "301 / 300", over: true });
});

test("the settings section is only live for managers of a public server", () => {
  const manager = { can_manage_server: true, public: true };
  assert.deepEqual(discoverySettingsState({ server: { ...manager, public: false }, listing: null }), {
    enabled: false, listed: false, hint: "Turn on Public server above to list it in Discover.",
  });
  assert.equal(discoverySettingsState({ server: { can_manage_server: false, public: true }, listing: null }).enabled, false);
  const fresh = discoverySettingsState({ server: manager, listing: null });
  assert.equal(fresh.enabled, true);
  assert.equal(fresh.listed, false);
  assert.match(fresh.hint, /rounded member count/);
  const listed = discoverySettingsState({ server: manager, listing: { code: "KQ7M3N4P" } });
  assert.equal(listed.listed, true);
  assert.match(listed.hint, /Listed in Discover/);
});
