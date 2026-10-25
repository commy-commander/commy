import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  MAX_TRUSTED_LINK_DOMAINS, LINK_SIGN_TEXT, readTrustedDomains, withTrustedDomain, writeTrustedDomains, shownHostParts, leavingPrompt, trustedLinksSummary,
} = require("../scripts/link-safety.js");

const check = over => ({ url: "https://www.example.org/", host: "www.example.org", shown_host: "www.example.org", site: "example.org", trust_key: "example.org", level: "ask", signs: [], can_trust: true, ...over });

test("trusted domains survive a round trip and ignore junk", () => {
  assert.deepEqual(readTrustedDomains(null), []);
  assert.deepEqual(readTrustedDomains("not json"), []);
  assert.deepEqual(readTrustedDomains('{"a":1}'), []);
  assert.deepEqual(readTrustedDomains('[" Example.org. ", 3, null, "example.org", "b.example"]'), ["example.org", "b.example"]);
  assert.deepEqual(readTrustedDomains(writeTrustedDomains(["a.example", "b.example"])), ["a.example", "b.example"]);
});

test("trusting a domain again moves it to the end, and the list keeps the newest", () => {
  assert.deepEqual(withTrustedDomain(["a.example", "b.example"], "A.example"), ["b.example", "a.example"]);
  assert.deepEqual(withTrustedDomain(["a.example"], ""), ["a.example"]);
  const many = Array.from({ length: MAX_TRUSTED_LINK_DOMAINS }, (_, i) => `site${i}.example`);
  const next = withTrustedDomain(many, "new.example");
  assert.equal(next.length, MAX_TRUSTED_LINK_DOMAINS);
  assert.equal(next[0], "site1.example");
  assert.equal(next.at(-1), "new.example");
});

test("the shown address keeps its subdomains visible and sets the site apart", () => {
  assert.deepEqual(shownHostParts(check()), { prefix: "www.", site: "example.org" });
  assert.deepEqual(shownHostParts(check({ shown_host: "paypal.com.evil.example", site: "evil.example" })), { prefix: "paypal.com.", site: "evil.example" });
  assert.deepEqual(shownHostParts(check({ shown_host: "example.org" })), { prefix: "", site: "example.org" });
  assert.deepEqual(shownHostParts(check({ shown_host: "10.0.0.1", site: "10.0.0.1", signs: ["address"] })), { prefix: "", site: "10.0.0.1" });
});

test("an unfamiliar site gets the plain prompt with the trust option", () => {
  const prompt = leavingPrompt(check());
  assert.equal(prompt.tone, "plain");
  assert.equal(prompt.title, "Leaving Commy");
  assert.equal(prompt.openLabel, "Open Site");
  assert.equal(prompt.canTrust, true);
  assert.deepEqual(prompt.notes, []);
  assert.equal(prompt.spelled, "");
});

test("lookalikes and mismatched text get the stronger warning without the trust option", () => {
  const prompt = leavingPrompt(check({ level: "warn", signs: ["text_mismatch", "lookalike"], can_trust: false }));
  assert.equal(prompt.tone, "warn");
  assert.equal(prompt.openLabel, "Open Anyway");
  assert.equal(prompt.canTrust, false);
  assert.deepEqual(prompt.notes, [LINK_SIGN_TEXT.text_mismatch, LINK_SIGN_TEXT.lookalike]);
});

test("international names show how they are spelled out", () => {
  const prompt = leavingPrompt(check({ level: "warn", host: "xn--mnchen-3ya.de", shown_host: "münchen.de", site: "xn--mnchen-3ya.de", signs: ["international"] }));
  assert.equal(prompt.spelled, "xn--mnchen-3ya.de");
  assert.deepEqual(prompt.host, { prefix: "", site: "münchen.de" });
});

test("listed sites are blocked, and unknown levels or signs fall back safely", () => {
  assert.equal(leavingPrompt(check({ level: "block", signs: ["known_bad"], can_trust: false })).tone, "block");
  const odd = leavingPrompt(check({ level: "mystery", signs: ["unheard_of"] }));
  assert.equal(odd.tone, "plain");
  assert.deepEqual(odd.notes, []);
});

test("no prompt text names a product, a protocol or a host", () => {
  const all = [...Object.values(LINK_SIGN_TEXT), ...["ask", "warn", "block"].flatMap(level => Object.values(leavingPrompt(check({ level }))).filter(v => typeof v === "string"))];
  for (const text of all) assert.doesNotMatch(text, /https?|www\.|\.com|\.org|dns|tls|url/i, text);
});

test("the settings summary counts trusted sites", () => {
  assert.equal(trustedLinksSummary([]), "Every site outside Commy asks before it opens.");
  assert.equal(trustedLinksSummary(["a.example"]), "1 site opens without asking on this device.");
  assert.equal(trustedLinksSummary(["a.example", "b.example"]), "2 sites open without asking on this device.");
});
