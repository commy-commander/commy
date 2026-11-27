import assert from "node:assert/strict";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";
import { EARLIER_DAYS, MAX_VAULT_BYTES, NEWEST_EARLIER_KEPT, cleanWriter, earlierToDrop, forgetItemsNoLongerKept, normalizeVaultName, validSealed } from "../src/vault.js";

const ORIGIN = "http://localhost:3001";
const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN };
const SEALED = (n) => "sealed1." + `Ab-_${n}`.padEnd(60, "x");

function makeAuth() {
  const options = buildAuthOptions(loadConfig(ENV), { pool: null, send: async () => {} });
  const db = { user: [], session: [], commyIdentity: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], inviteBackground: [], vaultItem: [], vaultEarlier: [], doorbellRing: [] };
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
  return { db, call, signUp, context: auth.$context };
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

test("the account server keeps no copy of the server list key", async () => {
  const { db, call, signUp } = makeAuth();
  const alice = await signUp("alice@example.test");
  const put = await call("POST", "/commy/vault/put", { cookie: alice, body: { name: "entry-key", data: SEALED(1), version: 0 } });
  assert.equal(put.status, 400);
  assert.equal(db.vaultItem.length, 0);
  assert.equal((await call("GET", "/commy/vault", { cookie: alice, query: { name: "entry-key" } })).json.data, null);
  assert.equal((await call("GET", "/commy/vault/earlier", { cookie: alice, query: { name: "entry-key" } })).status, 400);
});

test("copies of the server list key kept by earlier versions are cleared", async () => {
  const { db, call, signUp, context } = makeAuth();
  const alice = await signUp("alice@example.test");
  await putServers(call, alice, 1, 0);
  const userId = db.user[0].id;
  const at = new Date();
  db.vaultItem.push({ id: "old-key", userId, name: "entry-key", data: SEALED(9), version: 3, updatedAt: at });
  db.vaultEarlier.push({ id: "older-key", userId, name: "entry-key", data: SEALED(8), version: 2, updatedAt: at, replacedAt: at });
  await forgetItemsNoLongerKept((await context).adapter);
  assert.deepEqual(db.vaultItem.map((row) => row.name), ["servers"]);
  assert.equal(db.vaultEarlier.length, 0);
});

const DAY = 24 * 60 * 60 * 1000;

async function putServers(call, cookie, n, version, writer) {
  const r = await call("POST", "/commy/vault/put", { cookie, body: { name: "servers", data: SEALED(n), version, writer } });
  assert.equal(r.status, 200, JSON.stringify(r.json));
  return r.json.version;
}

test("discovery lists the earlier copies", () => {
  const d = discoveryDocument(loadConfig(ENV));
  assert.equal(d.endpoints.vaultEarlier, `${ORIGIN}/api/auth/commy/vault/earlier`);
  assert.equal(d.features.vaultEarlier, true);
});

test("every replaced server list is kept, newest first, with the build that wrote it", async () => {
  const { call, signUp } = makeAuth();
  const alice = await signUp("alice@example.test");
  let version = await putServers(call, alice, 1, 0, "Commy 0.2.4 (windows)");
  version = await putServers(call, alice, 2, version, "Commy 1.0.0-rc.2 (ios)");
  await putServers(call, alice, 3, version);

  const earlier = await call("GET", "/commy/vault/earlier", { cookie: alice, query: { name: "servers" } });
  assert.equal(earlier.status, 200, JSON.stringify(earlier.json));
  assert.deepEqual(
    earlier.json.items.map(({ version, data, writer }) => ({ version, data, writer })),
    [
      { version: 2, data: SEALED(2), writer: "Commy 1.0.0-rc.2 (ios)" },
      { version: 1, data: SEALED(1), writer: "Commy 0.2.4 (windows)" },
    ],
  );
  assert.ok(earlier.json.items.every((item) => item.replacedAt && item.updatedAt));
  assert.equal((await call("GET", "/commy/vault", { cookie: alice, query: { name: "servers" } })).json.data, SEALED(3));
});

test("a refused write keeps no copy and the current list stays", async () => {
  const { db, call, signUp } = makeAuth();
  const alice = await signUp("alice@example.test");
  await putServers(call, alice, 1, 0);
  assert.equal((await call("POST", "/commy/vault/put", { cookie: alice, body: { name: "servers", data: SEALED(2), version: 0 } })).status, 409);
  assert.equal(db.vaultEarlier.length, 0);
});

test("deleting the list keeps the last copy so it can be brought back", async () => {
  const { call, signUp } = makeAuth();
  const alice = await signUp("alice@example.test");
  await putServers(call, alice, 1, 0);
  await call("POST", "/commy/vault/delete", { cookie: alice, body: { name: "servers" } });
  const earlier = await call("GET", "/commy/vault/earlier", { cookie: alice, query: { name: "servers" } });
  assert.deepEqual(earlier.json.items.map((item) => item.data), [SEALED(1)]);
});

test("earlier copies stay with their account and only for kept items", async () => {
  const { call, signUp } = makeAuth();
  const alice = await signUp("alice@example.test");
  const bob = await signUp("bob@example.test");
  const version = await putServers(call, alice, 1, 0);
  await putServers(call, alice, 2, version);
  assert.equal((await call("GET", "/commy/vault/earlier", { query: { name: "servers" } })).status, 401);
  assert.deepEqual((await call("GET", "/commy/vault/earlier", { cookie: bob, query: { name: "servers" } })).json.items, []);
  assert.equal((await call("GET", "/commy/vault/earlier", { cookie: alice, query: { name: "read-state" } })).status, 400);
});

test("writer labels are short printable text", () => {
  assert.equal(cleanWriter(" Commy 1.0.0 (ios) "), "Commy 1.0.0 (ios)");
  assert.equal(cleanWriter("x".repeat(200)).length, 80);
  assert.equal(cleanWriter("a\u0000b\nc"), "abc");
  assert.equal(cleanWriter(undefined), null);
  assert.equal(cleanWriter(""), null);
});

function copies(count, { start, everyMs }) {
  return Array.from({ length: count }, (_, i) => ({ id: `c${i}`, version: i + 1, replacedAt: new Date(start + i * everyMs) }));
}

test("a burst of bad writes cannot push out the copy from before the burst", () => {
  const now = new Date("2026-11-19T12:00:00Z");
  const before = { id: "good", version: 1, replacedAt: new Date("2026-11-18T09:00:00Z") };
  const burst = copies(500, { start: Date.parse("2026-11-19T00:00:00Z"), everyMs: 1000 }).map((row) => ({ ...row, version: row.version + 1 }));
  const drop = new Set(earlierToDrop([before, ...burst], now));
  assert.ok(!drop.has("good"), "the last copy of the day before survives");
  assert.ok(!drop.has("c0"), "the first copy replaced in the bad day survives");
  const kept = 501 - drop.size;
  assert.equal(kept, NEWEST_EARLIER_KEPT + 2);
});

test("one copy a day is kept for the retention window, then dropped", () => {
  const now = new Date("2026-11-19T12:00:00Z");
  const daily = copies(80, { start: now.getTime() - 79 * DAY, everyMs: DAY });
  const drop = new Set(earlierToDrop(daily, now));
  const kept = daily.filter((row) => !drop.has(row.id));
  assert.equal(kept.length, EARLIER_DAYS + 1);
  assert.ok(kept.every((row) => now - row.replacedAt <= (EARLIER_DAYS + 1) * DAY));
});
