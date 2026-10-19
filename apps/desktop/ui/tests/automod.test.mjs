import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  AUTOMOD_MAX_TERMS, AUTOMOD_MAX_TERMS_BYTES, AUTOMOD_MAX_EXEMPT_ROLES,
  parseBlockedTerms, termsUsage, termsUsageText, burstKey, burstOptions, burstFromKey,
  mentionOptions, automodPayload, automodSummary, heldLabel,
} = require("../scripts/automod.js");

test("blocked words come one per line, trimmed, without repeats", () => {
  assert.deepEqual(parseBlockedTerms("  gold \n\nGold\nfree   nitro\n spam*"), ["gold", "free nitro", "spam*"]);
  assert.deepEqual(parseBlockedTerms(""), []);
  assert.deepEqual(parseBlockedTerms(null), []);
});

test("usage flags lists past the limits the backend enforces", () => {
  assert.equal(termsUsage(["a", "b"]).over, false);
  assert.equal(termsUsage(Array.from({ length: AUTOMOD_MAX_TERMS + 1 }, (_, i) => `w${i}`)).over, true);
  assert.equal(termsUsage(["x".repeat(41)]).over, true);
  assert.equal(termsUsage(Array.from({ length: 50 }, (_, i) => `${i}`.padStart(31, "x"))).over, true);
  assert.equal(termsUsageText(termsUsage(["ab", "cd"])), `2 / ${AUTOMOD_MAX_TERMS} words, 4 / ${AUTOMOD_MAX_TERMS_BYTES} characters`);
  assert.match(termsUsageText(termsUsage(["y".repeat(45)])), /^Too long: y+$/);
});

test("flood choices round trip and keep a setting made elsewhere", () => {
  assert.equal(burstKey(null), "off");
  assert.deepEqual(burstFromKey("5/10"), { messages: 5, seconds: 10 });
  assert.equal(burstFromKey("off"), null);
  const odd = { messages: 7, seconds: 15 };
  const options = burstOptions(odd);
  assert.equal(options[0].label, "Off");
  assert.deepEqual(options.at(-1), { value: "7/15", label: "More than 7 messages in 15 seconds" });
  assert.equal(burstOptions({ messages: 5, seconds: 10 }).length, burstOptions(null).length);
  assert.deepEqual(mentionOptions(7).map(o => o.value), ["0", "3", "5", "10", "20", "7"]);
  assert.equal(mentionOptions(0)[0].label, "Off");
});

test("the payload matches what the backend deserializes", () => {
  assert.deepEqual(
    automodPayload({ termsText: "gold\n", blockLinks: 1, mentionLimit: "5", burst: "3/5", exemptRoles: ["r1", "r1", "r2"] }),
    { blocked_terms: ["gold"], block_links: true, max_mentions: 5, burst: { messages: 3, seconds: 5 }, exempt_roles: ["r1", "r2"] },
  );
  assert.deepEqual(
    automodPayload({ termsText: "", blockLinks: false, mentionLimit: "0", burst: "off", exemptRoles: [] }),
    { blocked_terms: [], block_links: false, max_mentions: null, burst: null, exempt_roles: [] },
  );
  const rust = readFileSync(join(import.meta.dirname, "..", "..", "..", "..", "crates", "commy-core", "src", "automod.rs"), "utf8");
  for (const field of ["blocked_terms", "block_links", "max_mentions", "burst", "exempt_roles"]) {
    assert.match(rust, new RegExp(`pub ${field}:`), field);
  }
  assert.match(rust, new RegExp(`MAX_TERMS_BYTES: usize = ${AUTOMOD_MAX_TERMS_BYTES};`));
  assert.match(rust, new RegExp(`MAX_EXEMPT_ROLES: usize = ${AUTOMOD_MAX_EXEMPT_ROLES};`));
});

test("summaries and held labels read plainly", () => {
  assert.equal(automodSummary(null), "Off");
  assert.equal(automodSummary({ blocked_terms: [], block_links: false }), "Off");
  assert.equal(
    automodSummary({ blocked_terms: ["a"], block_links: true, max_mentions: 5, burst: { messages: 3, seconds: 5 } }),
    "1 blocked word, links blocked, mention limit 5, flood limit 3 in 5s",
  );
  assert.equal(heldLabel("blocked word"), "Held: blocked word");
  assert.equal(heldLabel(null), "Held");
});
