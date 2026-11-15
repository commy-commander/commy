import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { homeTagState, homeTagTitle, showsHomeInChat, homesToAsk, cleanHomeInput, withBlockedHome, withoutBlockedHome, MAX_BLOCKED_HOMES } = require("../scripts/home-tags.js");

const tag = over => ({ home: "https://commy.chat", shown: "commy.chat", key: "b62e867fa2f33afe", lookalike: false, current: true, ...over });
const status = over => ({ home: "https://commy.chat", key: "b62e867fa2f33afe", checked: true, blocked: false, ...over });

test("a home is verified only when its server publishes the card's key", () => {
  assert.equal(homeTagState(tag(), status()), "verified");
  assert.equal(homeTagState(tag(), status({ key: "0000000000000000" })), "unverified");
  assert.equal(homeTagState(tag(), status({ key: null })), "unverified");
  assert.equal(homeTagState(tag(), null), "checking");
  assert.equal(homeTagState(tag(), status({ checked: false })), "checking");
  assert.equal(homeTagState(null, status()), null);
});

test("blocked, look-alike and expired homes win over everything else", () => {
  assert.equal(homeTagState(tag({ lookalike: true }), status({ blocked: true })), "blocked");
  assert.equal(homeTagState(tag({ lookalike: true }), status()), "lookalike");
  assert.equal(homeTagState(tag({ current: false }), status()), "expired");
  assert.match(homeTagTitle(tag({ shown: "cornmy.chat" }), "lookalike"), /cornmy\.chat looks like/);
});

test("chat shows a home only when it is not yours or it needs a warning", () => {
  assert.equal(showsHomeInChat(tag(), "https://commy.chat", "verified"), false);
  assert.equal(showsHomeInChat(tag(), "https://commy.chat", "unverified"), true);
  assert.equal(showsHomeInChat(tag({ home: "https://friends.org" }), "https://commy.chat", "verified"), true);
  assert.equal(showsHomeInChat(null, "https://commy.chat", null), false);
});

test("each home is asked about once", () => {
  const members = [{ home: tag() }, { home: tag() }, { home: tag({ home: "https://friends.org" }) }, { home: null }, null];
  assert.deepEqual(homesToAsk(members, new Set()), ["https://commy.chat", "https://friends.org"]);
  assert.deepEqual(homesToAsk(members, new Set(["https://commy.chat"])), ["https://friends.org"]);
});

test("blocked server addresses are cleaned, kept sorted and capped", () => {
  assert.equal(cleanHomeInput(" https://Spam.Example/ "), "spam.example");
  for (const bad of ["", "localhost", "spam", "10.0.0.1x", "-bad.example"]) assert.equal(cleanHomeInput(bad), null, bad);
  assert.deepEqual(withBlockedHome(["b.example"], "a.example"), { homes: ["a.example", "b.example"], error: null });
  assert.deepEqual(withBlockedHome(["a.example"], "A.example"), { homes: ["a.example"], error: null });
  assert.match(withBlockedHome([], "nope").error, /server address/);
  const full = Array.from({ length: MAX_BLOCKED_HOMES }, (_, i) => `s${i}.example`);
  assert.match(withBlockedHome(full, "new.example").error, /up to/);
  assert.deepEqual(withoutBlockedHome(["a.example", "b.example"], "a.example"), ["b.example"]);
});
