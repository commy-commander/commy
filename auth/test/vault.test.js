import assert from "node:assert/strict";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";
import { MAX_VAULT_BYTES, normalizeVaultName, validSealed } from "../src/vault.js";

const ORIGIN = "http://localhost:3001";
const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN };
const SEALED = (n) => "sealed1." + `Ab-_${n}`.padEnd(60, "x");

function makeAuth() {
  const options = buildAuthOptions(loadConfig(ENV), { pool: null, send: async () => {} });
  const db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], inviteBackground: [], vaultItem: [], doorbellRing: [] };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  const auth = betterAuth(options);
  const call = async (method, path, { body, cookie, query } = {}) => {
    const url = new URL(`${ORIGIN}/api/auth${path}`);
    for (const [k, v] of Object.entries(query ?? {})) url.searchParams.set(k, v);
    const headers = cookie ? { origin: ORIGIN, cookie } : {};
    if (body) headers["content-type"] = "application/json";
    const res = await auth.handler(new Request(url, { method, headers, body: body ? JSON.stringify(body) : undefined }));
    const text = await res.text();
    return { status: res.status, json: text ? JSON.parse(text) : null, setCookie: res.headers.get("set-cookie") };
  };
  const signUp = async (email) => {
    const r = await call("POST", "/sign-up/email", { cookie: "x=1", body: { email, password: "correct horse battery", name: "T" } });
    assert.equal(r.status, 200, JSON.stringify(r.json));
    return r.setCookie.split(";")[0];
  };
  return { db, call, signUp };
}

test("names and sealed data are validated", () => {
  assert.equal(normalizeVaultName("servers"), "servers");
  for (const bad of ["", "Servers", "../x", "a".repeat(33), null]) assert.equal(normalizeVaultName(bad), null);
  assert.equal(validSealed(SEALED(1)), SEALED(1));
  assert.equal(validSealed('{"servers":["Rust Club"]}'), null, "only sealed data is stored");
  assert.equal(validSealed("sealed1." + "a".repeat(MAX_VAULT_BYTES)), null);
});

test("discovery lists the vault", () => {
  const d = discoveryDocument(loadConfig(ENV));
  assert.equal(d.endpoints.vault, `${ORIGIN}/api/auth/commy/vault`);
  assert.equal(d.features.vault, true);
});

test("items need a session and stay with their account", async () => {
  const { call, signUp } = makeAuth();
  assert.equal((await call("GET", "/commy/vault", { query: { name: "servers" } })).status, 401);
  assert.equal((await call("POST", "/commy/vault/put", { body: { name: "servers", data: SEALED(1), version: 0 } })).status, 401);

  const alice = await signUp("alice@example.test");
  const bob = await signUp("bob@example.test");
  const empty = await call("GET", "/commy/vault", { cookie: alice, query: { name: "servers" } });
  assert.deepEqual(empty.json, { name: "servers", data: null, version: 0, updatedAt: null });

  const put = await call("POST", "/commy/vault/put", { cookie: alice, body: { name: "servers", data: SEALED(1), version: 0 } });
  assert.equal(put.status, 200, JSON.stringify(put.json));
  assert.equal(put.json.version, 1);
  assert.equal((await call("GET", "/commy/vault", { cookie: bob, query: { name: "servers" } })).json.data, null);
  assert.equal((await call("GET", "/commy/vault", { cookie: alice, query: { name: "servers" } })).json.data, SEALED(1));
});

test("writes must name the version they replace", async () => {
  const { db, call, signUp } = makeAuth();
  const alice = await signUp("alice@example.test");
  await call("POST", "/commy/vault/put", { cookie: alice, body: { name: "servers", data: SEALED(1), version: 0 } });
  const stale = await call("POST", "/commy/vault/put", { cookie: alice, body: { name: "servers", data: SEALED(2), version: 0 } });
  assert.equal(stale.status, 409);
  assert.equal(stale.json.conflict, true);
  assert.equal(stale.json.version, 1);
  assert.equal(stale.json.data, SEALED(1), "the conflict hands back the newer item to merge");

  const ok = await call("POST", "/commy/vault/put", { cookie: alice, body: { name: "servers", data: SEALED(2), version: 1 } });
  assert.equal(ok.json.version, 2);
  assert.equal(db.vaultItem.length, 1);

  const bad = await call("POST", "/commy/vault/put", { cookie: alice, body: { name: "servers", data: "Rust Club", version: 2 } });
  assert.equal(bad.status, 400);

  await call("POST", "/commy/vault/delete", { cookie: alice, body: { name: "servers" } });
  assert.equal(db.vaultItem.length, 0);
});
