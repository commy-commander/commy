import assert from "node:assert/strict";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";
import { folderKeyDeriver, normalizeLockId } from "../src/folder-lock.js";

const ORIGIN = "http://localhost:3001";
const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN };
const LOCK = "0123456789abcdef0123456789abcdef";

function makeAuth(env = ENV) {
  const options = buildAuthOptions(loadConfig(env), { pool: null, send: async () => {} });
  const db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], inviteBackground: [], vaultItem: [], doorbellRing: [] };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  const auth = betterAuth(options);
  const call = async (path, { body, cookie } = {}) => {
    const headers = { "content-type": "application/json", ...(cookie ? { origin: ORIGIN, cookie } : {}) };
    const res = await auth.handler(new Request(`${ORIGIN}/api/auth${path}`, { method: "POST", headers, body: JSON.stringify(body ?? {}) }));
    const text = await res.text();
    return { status: res.status, json: text ? JSON.parse(text) : null, setCookie: res.headers.get("set-cookie") };
  };
  const signUp = async (email) => {
    const r = await call("/sign-up/email", { cookie: "x=1", body: { email, password: "correct horse battery", name: "T" } });
    assert.equal(r.status, 200, JSON.stringify(r.json));
    return r.setCookie.split(";")[0];
  };
  return { db, call, signUp };
}

test("lock ids are 32 hex characters", () => {
  assert.equal(normalizeLockId(` ${LOCK.toUpperCase()} `), LOCK);
  for (const bad of ["", "abc", LOCK + "0", "g".repeat(32), null]) assert.equal(normalizeLockId(bad), null);
});

test("discovery offers folder locks", () => {
  const d = discoveryDocument(loadConfig(ENV));
  assert.equal(d.endpoints.folderKey, `${ORIGIN}/api/auth/commy/folder-key`);
  assert.equal(d.features.folderLock, true);
});

test("a folder key needs a session and belongs to one account and one folder", async () => {
  const { db, call, signUp } = makeAuth();
  assert.equal((await call("/commy/folder-key", { body: { lockId: LOCK } })).status, 401);

  const alice = await signUp("alice@example.test");
  const bob = await signUp("bob@example.test");
  const first = await call("/commy/folder-key", { cookie: alice, body: { lockId: LOCK } });
  assert.equal(first.status, 200);
  assert.match(first.json.key, /^[0-9a-f]{64}$/);
  const again = await call("/commy/folder-key", { cookie: alice, body: { lockId: LOCK } });
  assert.equal(again.json.key, first.json.key, "the same folder always gets the same key");

  const other = await call("/commy/folder-key", { cookie: alice, body: { lockId: "f".repeat(32) } });
  assert.notEqual(other.json.key, first.json.key);
  const bobs = await call("/commy/folder-key", { cookie: bob, body: { lockId: LOCK } });
  assert.notEqual(bobs.json.key, first.json.key, "another account cannot get this folder's key");

  assert.equal((await call("/commy/folder-key", { cookie: alice, body: { lockId: "nope" } })).status, 400);
  assert.ok(!JSON.stringify(db).includes(first.json.key), "nothing is stored");
});

test("the key depends on the server secret", () => {
  const a = folderKeyDeriver("secret-a".repeat(4));
  const b = folderKeyDeriver("secret-b".repeat(4));
  assert.equal(a("u1", LOCK), a("u1", LOCK));
  assert.notEqual(a("u1", LOCK), b("u1", LOCK));
  assert.notEqual(a("u1", LOCK), a("u2", LOCK));
});
