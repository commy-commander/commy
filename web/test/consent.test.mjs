import assert from "node:assert/strict";
import { test } from "node:test";
import { CONSENT_COOKIE, parseConsent } from "../lib/consent.ts";

test("no cookie means no decision yet", () => {
  assert.equal(parseConsent(""), null);
  assert.equal(parseConsent(null), null);
  assert.equal(parseConsent("other=1; theme=dark"), null);
});

test("reads an accept or reject decision", () => {
  assert.equal(parseConsent(`${CONSENT_COOKIE}=accepted`), "accepted");
  assert.equal(parseConsent(`a=1; ${CONSENT_COOKIE}=rejected; b=2`), "rejected");
});

test("ignores unknown values and look-alike names", () => {
  assert.equal(parseConsent(`${CONSENT_COOKIE}=yes`), null);
  assert.equal(parseConsent(`x${CONSENT_COOKIE}=accepted`), null);
});
