import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  BAN_LENGTHS, UPLOAD_LIMITS_MB, uploadLimitLabel, uploadRefusal, spanWords, sinceWords, banLine, kickLine, banChoiceText, canSeeRemoved,
} = require("../scripts/moderation.js");

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const rustList = (file, name) => {
  const source = readFileSync(join(repo, "crates", "commy-core", "src", file), "utf8");
  return new RegExp(`${name}: &\\[u32\\] = &\\[([^\\]]*)\\]`).exec(source)[1].split(",").map(n => Number(n.trim()));
};

test("ban lengths and file limits match what the core accepts", () => {
  assert.deepEqual(BAN_LENGTHS.filter(b => b.hours).map(b => b.hours), rustList("removals.rs", "BAN_HOURS"));
  assert.deepEqual(UPLOAD_LIMITS_MB, rustList("server.rs", "UPLOAD_LIMITS_MB"));
  assert.equal(BAN_LENGTHS.at(-1).hours, null);
  for (const label of ["5 hours", "1 day", "1 week", "1 month", "Until unbanned"]) assert.ok(BAN_LENGTHS.some(b => b.label === label), label);
});

test("file limits read in megabytes or whole gigabytes", () => {
  assert.equal(uploadLimitLabel(null), "Up to 4 GB");
  assert.equal(uploadLimitLabel(25), "25 MB");
  assert.equal(uploadLimitLabel(1024), "1 GB");
  assert.equal(uploadLimitLabel(2048), "2 GB");
});

test("a file over the server's limit is refused before it uploads", () => {
  const server = { upload_limit_mb: 25 };
  assert.equal(uploadRefusal(server, 25 * 1024 * 1024), null);
  assert.equal(uploadRefusal(server, 25 * 1024 * 1024 + 1), "This server allows files up to 25 MB");
  assert.equal(uploadRefusal({ upload_limit_mb: null }, 4e9), null);
  assert.equal(uploadRefusal(null, 4e9), null);
});

test("lengths read in the largest whole unit", () => {
  assert.equal(spanWords(30_000), "1 minute");
  assert.equal(spanWords(5 * 3600_000), "5 hours");
  assert.equal(spanWords(36 * 3600_000), "1 day");
  assert.equal(spanWords(15 * 86400_000), "2 weeks");
  assert.equal(sinceWords(0, 10), null);
  assert.equal(sinceWords(1_000, 20_000), "just now");
  assert.equal(sinceWords(1_000, 1_000 + 2 * 3600_000), "2 hours ago");
});

test("removed people show who removed them, when, and when a ban ends", () => {
  const now = 100 * 86400_000;
  const hour = 3600_000;
  assert.equal(banLine({ by_name: "Mia", at: now - 2 * hour, until: now + 5 * hour }, now), "Banned by Mia 2 hours ago, ends in 5 hours");
  assert.equal(banLine({ by_name: null, at: 0, until: null }, now), "Banned, until unbanned");
  assert.equal(kickLine({ by_name: "Theo", at: now - 3 * 86400_000 }, now), "Kicked by Theo 3 days ago");
});

test("the ban dialog says when they can come back", () => {
  assert.equal(banChoiceText("Theo", "Hangout", null), "Theo is removed from Hangout and cannot rejoin until you unban them.");
  assert.equal(banChoiceText("Theo", "Hangout", 24), "Theo is removed from Hangout and can rejoin with an invite after 1 day.");
});

test("only people who can kick or ban see the list", () => {
  assert.equal(canSeeRemoved({ can_ban: true }), true);
  assert.equal(canSeeRemoved({ can_kick: true }), true);
  assert.equal(canSeeRemoved({}), false);
  assert.equal(canSeeRemoved(null), false);
});
