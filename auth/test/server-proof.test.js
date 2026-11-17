import assert from "node:assert/strict";
import { createPublicKey, verify } from "node:crypto";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";
import { keyFromSeed, signedBytes } from "../src/server-key.js";
import { PROOF_PURPOSE, proveServer } from "../src/server-proof.js";

const ORIGIN = "http://localhost:3001";
const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN };
const SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
const NONCE = "ab".repeat(32);

function signedBy(publicHex, bytes, sigHex) {
  const key = createPublicKey({ key: Buffer.concat([SPKI_PREFIX, Buffer.from(publicHex, "hex")]), format: "der", type: "spki" });
  return verify(null, bytes, key, Buffer.from(sigHex, "hex"));
}

test("a proof signs the caller's nonce over the server's own origin", () => {
  const key = keyFromSeed(Buffer.alloc(32, 7));
  const proof = proveServer(key, "https://commy.chat", NONCE.toUpperCase());
  assert.equal(proof.nonce, NONCE);
  assert.deepEqual(proof.key, key.published);
  assert.ok(signedBy(key.published.public, signedBytes(PROOF_PURPOSE, "https://commy.chat", NONCE), proof.sig));
  assert.ok(!signedBy(key.published.public, signedBytes(PROOF_PURPOSE, "https://cornmy.chat", NONCE), proof.sig));
  assert.equal(proveServer(key, "https://commy.chat", "short"), null);
  assert.equal(proveServer(key, "https://commy.chat", "zz".repeat(32)), null);
});

test("discovery points at the proof endpoint, which answers without a session", async () => {
  const config = loadConfig(ENV);
  assert.equal(discoveryDocument(config).endpoints.serverProof, `${ORIGIN}/api/auth/commy/server-proof`);
  const options = buildAuthOptions(config, { pool: null, send: async () => {} });
  options.database = sealedAtRest(memoryAdapter({ user: [], session: [], account: [], verification: [] }), options.secret);
  options.rateLimit = { enabled: false };
  const auth = betterAuth(options);
  const get = (nonce) => auth.handler(new Request(`${ORIGIN}/api/auth/commy/server-proof?nonce=${nonce}`));
  const res = await get(NONCE);
  assert.equal(res.status, 200);
  assert.equal(res.headers.get("cache-control"), "no-store");
  const body = await res.json();
  assert.equal(body.origin, ORIGIN);
  assert.deepEqual(body.key, config.serverKey.published);
  assert.ok(signedBy(body.key.public, signedBytes(PROOF_PURPOSE, ORIGIN, NONCE), body.sig));
  assert.equal((await get("nope")).status, 400);
});
