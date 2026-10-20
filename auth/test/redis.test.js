import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { createServer } from "node:net";
import { after, before, test } from "node:test";
import { setTimeout as delay } from "node:timers/promises";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { buildAuthOptions } from "../src/auth.js";
import { loadConfig } from "../src/config.js";
import { MAX_AGE_MS, MAX_PER_TOKEN, REDIS_PREFIX, commyDoorbell } from "../src/doorbell.js";
import { rateLimitKey } from "../src/rate-limit.js";
import { connectRedis } from "../src/redis.js";

const ORIGIN = "http://localhost:3001";
const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN };
const newToken = () => randomBytes(16).toString("hex");
const newId = () => randomBytes(12).toString("hex");
const sorted = (list) => [...list].sort();

let server = null;
let redis = null;

async function freePort() {
  return new Promise((resolve, reject) => {
    const probe = createServer();
    probe.once("error", reject);
    probe.listen(0, "127.0.0.1", () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });
}

async function startRedis() {
  if (process.env.REDIS_TEST_URL) return process.env.REDIS_TEST_URL;
  const port = await freePort();
  server = spawn("redis-server", ["--port", String(port), "--bind", "127.0.0.1", "--save", "", "--appendonly", "no"], {
    stdio: "ignore",
  });
  const failed = new Promise((resolve) => server.once("error", () => resolve(null)));
  const ready = (async () => {
    for (let i = 0; i < 100; i++) {
      await delay(50);
      const probe = connectRedis(`redis://127.0.0.1:${port}`, { log: { warn() {} } });
      const up = await Promise.race([probe.ready.then(() => true), delay(200).then(() => false)]);
      await probe.close().catch(() => {});
      if (up) return `redis://127.0.0.1:${port}`;
    }
    return null;
  })();
  return Promise.race([failed, ready]);
}

before(async () => {
  const url = await startRedis();
  if (!url) return;
  redis = connectRedis(url, { log: { warn() {} } });
  await redis.ready;
});

after(async () => {
  await redis?.close();
  server?.kill();
});

const SKIP = "no Redis: set REDIS_TEST_URL or install redis-server";

async function makeAuth({ doorbell, rateLimit = false, shared = redis } = {}) {
  const options = buildAuthOptions(loadConfig(ENV), { pool: null, send: async () => {}, redis: shared });
  if (doorbell) options.plugins = options.plugins.map((p) => (p.id === "commy-doorbell" ? commyDoorbell({ redis: shared, ...doorbell }) : p));
  const db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], inviteBackground: [], doorbellRing: [] };
  options.database = memoryAdapter(db);
  if (!rateLimit) options.rateLimit = { enabled: false };
  const auth = betterAuth(options);
  await auth.handler(new Request(`${ORIGIN}/api/auth/ok`));
  await delay(20);
  const call = async (path, body, headers = {}) => {
    const res = await auth.handler(
      new Request(`${ORIGIN}/api/auth${path}`, { method: "POST", headers: { "content-type": "application/json", ...headers }, body: JSON.stringify(body) }),
    );
    const text = await res.text();
    return { status: res.status, json: text ? JSON.parse(text) : null };
  };
  const ring = (token, ids, headers) => call("/commy/doorbell/ring", { token, ids }, headers);
  const take = (token, wait, headers) => call("/commy/doorbell/take", wait === undefined ? { token } : { token, wait }, headers);
  return { db, call, ring, take };
}

const fresh = async () => {
  await redis.client.flushDb();
};
const stored = (token) => redis.client.zRangeWithScores(`${REDIS_PREFIX}t:${token}`, 0, -1);
const total = async () => Number((await redis.client.get(`${REDIS_PREFIX}total`)) ?? 0);
const allKeys = () => redis.client.keys("*");

test("ring then take returns the ids once and the database table stays empty", async (t) => {
  if (!redis) return t.skip(SKIP);
  await fresh();
  const { db, ring, take } = await makeAuth();
  const token = newToken();
  const ids = [newId(), newId()];
  const r = await ring(token, ids);
  assert.equal(r.status, 200, JSON.stringify(r.json));
  assert.equal((await stored(token)).length, 2);
  assert.equal(await total(), 2);
  assert.equal(db.doorbellRing.length, 0, "nothing goes to the database");

  const first = await take(token, 0);
  assert.deepEqual(sorted(first.json.ids), sorted(ids));
  assert.equal(first.json.hello, false);
  assert.deepEqual(await stored(token), []);
  assert.equal(await total(), 0);
  assert.deepEqual((await take(token)).json, { ids: [], hello: false });
});

test("rings are kept per token and repeated ids are stored once", async (t) => {
  if (!redis) return t.skip(SKIP);
  await fresh();
  const { ring, take } = await makeAuth();
  const a = newToken();
  const b = newToken();
  const id = newId();
  const other = newId();
  await ring(a, [id, id]);
  await ring(a, [id, other]);
  await ring(b, [newId()]);
  assert.equal((await stored(a)).length, 2);
  assert.equal(await total(), 3);
  assert.deepEqual(sorted((await take(a, 0)).json.ids), sorted([id, other]));
  assert.equal((await stored(b)).length, 1, "another token's ids stay");
  assert.equal(await total(), 1);
});

test("Redis holds only tokens, blinded ids and times", async (t) => {
  if (!redis) return t.skip(SKIP);
  await fresh();
  const { ring, take } = await makeAuth();
  const token = newToken();
  const id = newId();
  const headers = { "user-agent": "commy-test/9.9", "x-forwarded-for": "192.0.2.77", authorization: "Bearer not-a-session" };
  await ring(token, [id], headers);
  await take(newToken(), 0, headers);
  const keys = await allKeys();
  assert.deepEqual(sorted(keys), sorted([`${REDIS_PREFIX}t:${token}`, `${REDIS_PREFIX}tokens`, `${REDIS_PREFIX}total`]));
  const [entry] = await stored(token);
  assert.equal(entry.value, id);
  assert.ok(Math.abs(entry.score - Date.now()) < 5000, "the score is the arrival time");
  const dump = JSON.stringify([keys, await stored(token), await redis.client.zRangeWithScores(`${REDIS_PREFIX}tokens`, 0, -1)]);
  for (const caller of ["192.0.2.77", "commy-test", "not-a-session"]) assert.ok(!dump.includes(caller));
});

test("a hello is never stored but wakes a waiting take", async (t) => {
  if (!redis) return t.skip(SKIP);
  await fresh();
  const { ring, take } = await makeAuth();
  const token = newToken();
  assert.deepEqual((await ring(token, [])).json, { ok: true });
  assert.deepEqual(await allKeys(), []);
  const started = Date.now();
  const waiting = take(token, 5);
  await delay(50);
  await ring(token, []);
  assert.deepEqual((await waiting).json, { ids: [], hello: true });
  assert.ok(Date.now() - started < 2000);
});

test("a ring on one auth instance wakes a take waiting on another", async (t) => {
  if (!redis) return t.skip(SKIP);
  await fresh();
  const one = await makeAuth();
  const two = await makeAuth();
  const token = newToken();
  const id = newId();
  const started = Date.now();
  const waiting = one.take(token, 5);
  await delay(50);
  await two.ring(token, [id]);
  assert.deepEqual((await waiting).json, { ids: [id], hello: false });
  assert.ok(Date.now() - started < 2000);
  assert.deepEqual(await stored(token), []);
});

test("two takes waiting on one token never both get the same ids", async (t) => {
  if (!redis) return t.skip(SKIP);
  await fresh();
  const { ring, take } = await makeAuth();
  const token = newToken();
  const ids = [newId(), newId(), newId()];
  const first = take(token, 0.5);
  const second = take(token, 0.5);
  await delay(50);
  await ring(token, ids);
  const got = (await Promise.all([first, second])).flatMap((r) => r.json.ids);
  assert.deepEqual(sorted(got), sorted(ids));
});

test("ids rung while takes read are all delivered exactly once", async (t) => {
  if (!redis) return t.skip(SKIP);
  await fresh();
  const { ring, take } = await makeAuth();
  const token = newToken();
  const ids = Array.from({ length: 40 }, newId);
  const rings = ids.map((id) => ring(token, [id]));
  const takes = Array.from({ length: 5 }, () => take(token, 0));
  await Promise.all(rings);
  const taken = (await Promise.all(takes)).flatMap((r) => r.json.ids);
  const rest = (await take(token, 0)).json.ids;
  assert.deepEqual(sorted([...taken, ...rest]), sorted(ids));
  assert.equal(await total(), 0);
});

test("a token keeps at most 1000 ids, dropping the oldest", async (t) => {
  if (!redis) return t.skip(SKIP);
  await fresh();
  const { ring, take } = await makeAuth();
  const token = newToken();
  const start = Date.now() - 60 * 60 * 1000;
  const old = Array.from({ length: MAX_PER_TOKEN }, (_, i) => ({ score: start + i * 1000, value: newId() }));
  await redis.client.zAdd(`${REDIS_PREFIX}t:${token}`, old);
  await redis.client.set(`${REDIS_PREFIX}total`, String(MAX_PER_TOKEN));
  const freshIds = [newId(), newId(), newId()];
  await ring(token, freshIds);
  const rows = await stored(token);
  assert.equal(rows.length, MAX_PER_TOKEN);
  const kept = new Set(rows.map((row) => row.value));
  for (const gone of old.slice(0, 3)) assert.ok(!kept.has(gone.value), "the oldest were dropped");
  assert.ok(kept.has(old[3].value));
  for (const id of freshIds) assert.ok(kept.has(id));
  assert.equal(await total(), MAX_PER_TOKEN, "the overall count follows the cap");
  assert.equal((await take(token, 0)).json.ids.length, MAX_PER_TOKEN);
  assert.equal(await total(), 0);
});

test("rings are refused once the overall cap is reached", async (t) => {
  if (!redis) return t.skip(SKIP);
  await fresh();
  const { ring, take } = await makeAuth({ doorbell: { maxRows: 2 } });
  const a = newToken();
  assert.equal((await ring(a, [newId(), newId()])).status, 200);
  assert.equal((await ring(newToken(), [newId()])).status, 503);
  assert.equal((await ring(newToken(), [])).status, 200, "hellos store nothing and still go through");
  await take(a, 0);
  assert.equal((await ring(newToken(), [newId()])).status, 200, "takes make room");
});

test("ids past 14 days are dropped by a take and by the sweep", async (t) => {
  if (!redis) return t.skip(SKIP);
  await fresh();
  const now = Date.now();
  const token = newToken();
  const keep = newId();
  await redis.client.zAdd(`${REDIS_PREFIX}t:${token}`, [
    { score: now - MAX_AGE_MS - 60_000, value: newId() },
    { score: now - 1000, value: keep },
  ]);
  await redis.client.zAdd(`${REDIS_PREFIX}tokens`, [{ score: now - 1000, value: token }]);
  await redis.client.set(`${REDIS_PREFIX}total`, "2");
  const { take } = await makeAuth();
  assert.deepEqual((await take(token, 0)).json.ids, [keep]);
  assert.equal(await total(), 0);

  const stale = newToken();
  const live = newToken();
  await redis.client.zAdd(`${REDIS_PREFIX}t:${stale}`, [{ score: now - MAX_AGE_MS - 60_000, value: newId() }]);
  await redis.client.zAdd(`${REDIS_PREFIX}t:${live}`, [{ score: now - 1000, value: newId() }]);
  await redis.client.zAdd(`${REDIS_PREFIX}tokens`, [
    { score: now - MAX_AGE_MS - 60_000, value: stale },
    { score: now - 1000, value: live },
  ]);
  await redis.client.set(`${REDIS_PREFIX}total`, "2");
  await makeAuth({ doorbell: { firstSweepMs: 5, sweepEveryMs: 60_000 } });
  for (let i = 0; i < 100 && (await stored(stale)).length > 0; i++) await delay(10);
  assert.deepEqual(await stored(stale), []);
  assert.equal((await stored(live)).length, 1);
  assert.deepEqual(await redis.client.zRange(`${REDIS_PREFIX}tokens`, 0, -1), [live]);
  assert.equal(await total(), 1);
});

test("rate limits count in Redis under keyed hashes, never an address", async (t) => {
  if (!redis) return t.skip(SKIP);
  await fresh();
  const { take } = await makeAuth({ rateLimit: true });
  const ip = "198.51.100.41";
  const headers = { "x-forwarded-for": ip };
  const token = newToken();
  for (let i = 0; i < 600; i++) assert.equal((await take(token, 0, headers)).status, 200, `take ${i}`);
  const limited = await take(token, 0, headers);
  assert.equal(limited.status, 429);
  assert.equal((await take(token, 0, { "x-forwarded-for": "198.51.100.42" })).status, 200, "other IPs are not limited");

  const keys = (await allKeys()).filter((key) => key.startsWith("rl:"));
  assert.ok(keys.length >= 2, "one counter per IP and path");
  for (const key of keys) {
    assert.match(key, /^rl:[A-Za-z0-9_-]{22}$/);
    assert.ok(!key.includes("198.51.100"));
    const ttl = await redis.client.ttl(key);
    assert.ok(ttl > 0 && ttl <= 60, `counter expires with its window (${ttl})`);
  }
  assert.ok(keys.includes(rateLimitKey(ENV.BETTER_AUTH_SECRET, `${ip}|/commy/doorbell/take`)), "the key is the hash of IP and path");
  assert.equal(await redis.client.get(rateLimitKey(ENV.BETTER_AUTH_SECRET, `${ip}|/commy/doorbell/take`)), "601");

  const other = await makeAuth({ rateLimit: true });
  assert.equal((await other.take(token, 0, headers)).status, 429);
});

test("without Redis the rate limiter keeps counting in memory", async (t) => {
  if (!redis) return t.skip(SKIP);
  const options = buildAuthOptions(loadConfig(ENV), { pool: null, send: async () => {}, redis: null });
  assert.equal(options.rateLimit.customStorage, undefined);
  const withRedis = buildAuthOptions(loadConfig(ENV), { pool: null, send: async () => {}, redis });
  assert.equal(typeof withRedis.rateLimit.customStorage.consume, "function");
});
