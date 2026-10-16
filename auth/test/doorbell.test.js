// Message doorbell: rings store blinded ids per token, takes return and delete
// them, and a waiting take wakes on a ring or an empty hello.
import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { test } from "node:test";
import { setTimeout as delay } from "node:timers/promises";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, featureSummary, loadConfig } from "../src/config.js";
import { MAX_AGE_MS, MAX_BODY_BYTES, MAX_IDS, MAX_PER_TOKEN, commyDoorbell } from "../src/doorbell.js";

const ORIGIN = "http://localhost:3001";
const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN };
const DAY_MS = 24 * 60 * 60 * 1000;

const newToken = () => randomBytes(16).toString("hex");
const newId = () => randomBytes(12).toString("hex");
const sorted = (list) => [...list].sort();

// `doorbell` swaps in a plugin built with those options, for timer and cap tests
function makeAuth({ env = {}, doorbell, rateLimit = false } = {}) {
  const options = buildAuthOptions(loadConfig({ ...ENV, ...env }), { pool: null, send: async () => {} });
  if (doorbell) options.plugins = options.plugins.map((p) => (p.id === "commy-doorbell" ? commyDoorbell(doorbell) : p));
  const db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], doorbellRing: [] };
  options.database = memoryAdapter(db);
  if (!rateLimit) options.rateLimit = { enabled: false };
  const auth = betterAuth(options);
  // The desktop app sends no Origin header and no cookies
  const send = async (path, init) => {
    const res = await auth.handler(new Request(`${ORIGIN}/api/auth${path}`, { method: "POST", ...init }));
    const text = await res.text();
    return { status: res.status, json: text ? JSON.parse(text) : null };
  };
  const call = (path, body, { headers = {}, signal } = {}) =>
    send(path, { headers: { "content-type": "application/json", ...headers }, body: JSON.stringify(body), signal });
  const ring = (token, ids, opts) => call("/commy/doorbell/ring", { token, ids }, opts);
  const take = (token, wait, opts) => call("/commy/doorbell/take", wait === undefined ? { token } : { token, wait }, opts);
  return { db, send, call, ring, take };
}

function rowsFor(db, token) {
  return db.doorbellRing.filter((row) => row.token === token);
}

test("discovery advertises the doorbell and DOORBELL=false turns it off", async () => {
  const on = loadConfig(ENV);
  assert.equal(on.doorbell, true);
  const d = discoveryDocument(on);
  assert.equal(d.features.doorbell, true);
  assert.equal(d.endpoints.doorbellRing, `${ORIGIN}/api/auth/commy/doorbell/ring`);
  assert.equal(d.endpoints.doorbellTake, `${ORIGIN}/api/auth/commy/doorbell/take`);
  assert.equal(new URL(d.endpoints.doorbellRing).origin, ORIGIN, "the app requires a same-origin ring endpoint");
  assert.match(featureSummary(on), /doorbell=on/);

  const off = loadConfig({ ...ENV, DOORBELL: "false" });
  assert.equal(off.doorbell, false);
  assert.equal(discoveryDocument(off).features.doorbell, false);
  assert.match(featureSummary(off), /doorbell=off/);
  assert.equal(loadConfig({ ...ENV, DOORBELL: "true" }).doorbell, true);

  const { ring, take } = makeAuth({ env: { DOORBELL: "false" } });
  assert.equal((await ring(newToken(), [newId()])).status, 404);
  assert.equal((await take(newToken(), 0)).status, 404);
});

test("ring then take returns the ids once and deletes them", async () => {
  const { db, ring, take } = makeAuth();
  const token = newToken();
  const ids = [newId(), newId()];
  const r = await ring(token, ids);
  assert.equal(r.status, 200, JSON.stringify(r.json));
  assert.deepEqual(r.json, { ok: true });
  assert.equal(rowsFor(db, token).length, 2);

  const first = await take(token, 0);
  assert.equal(first.status, 200, JSON.stringify(first.json));
  assert.deepEqual(sorted(first.json.ids), sorted(ids));
  assert.equal(first.json.hello, false);
  assert.equal(db.doorbellRing.length, 0, "take deletes what it returns");

  const second = await take(token);
  assert.deepEqual(second.json, { ids: [], hello: false });
});

test("rings are kept per token and repeated ids are stored once", async () => {
  const { db, ring, take } = makeAuth();
  const a = newToken();
  const b = newToken();
  const id = newId();
  const other = newId();
  await ring(a, [id, id]);
  await ring(a, [id, other]);
  await ring(b, [newId()]);
  assert.equal(rowsFor(db, a).length, 2);

  assert.deepEqual(sorted((await take(a, 0)).json.ids), sorted([id, other]));
  assert.equal(rowsFor(db, b).length, 1, "another token's ids stay");
  assert.deepEqual((await take(newToken(), 0)).json.ids, []);
});

test("rows hold only the token, the blinded id and the time", async () => {
  const { db, ring } = makeAuth();
  const token = newToken();
  await ring(token, [newId()], {
    headers: { "user-agent": "commy-test/9.9", "x-forwarded-for": "192.0.2.77", authorization: "Bearer not-a-session" },
  });
  assert.equal(db.doorbellRing.length, 1);
  assert.deepEqual(Object.keys(db.doorbellRing[0]).sort(), ["blindedId", "createdAt", "id", "token"]);
  const stored = JSON.stringify(db);
  for (const caller of ["192.0.2.77", "commy-test", "not-a-session"]) assert.ok(!stored.includes(caller));
});

test("a hello is never stored but wakes a waiting take", async () => {
  const { db, ring, take } = makeAuth();
  const token = newToken();

  // Nobody is listening: the hello is dropped. Without the table any read or
  // write would fail, so this also shows a hello never touches it.
  const table = db.doorbellRing;
  delete db.doorbellRing;
  const dropped = await ring(token, []);
  db.doorbellRing = table;
  assert.equal(dropped.status, 200, JSON.stringify(dropped.json));
  assert.deepEqual(dropped.json, { ok: true });
  assert.equal(db.doorbellRing.length, 0);
  assert.deepEqual((await take(token, 0)).json, { ids: [], hello: false });

  const started = Date.now();
  const waiting = take(token, 5);
  await delay(50);
  assert.deepEqual((await ring(token, [])).json, { ok: true });
  const woken = await waiting;
  assert.equal(woken.status, 200, JSON.stringify(woken.json));
  assert.deepEqual(woken.json, { ids: [], hello: true });
  assert.ok(Date.now() - started < 2000, "the hello woke the take before its wait ended");
  assert.equal(db.doorbellRing.length, 0);
});

test("a ring wakes a waiting take with its ids", async () => {
  const { db, ring, take } = makeAuth();
  const token = newToken();
  const id = newId();
  const started = Date.now();
  const waiting = take(token, 5);
  await delay(50);
  await ring(newToken(), [newId()]);
  await ring(token, [id]);
  const woken = await waiting;
  assert.deepEqual(woken.json, { ids: [id], hello: false });
  assert.ok(Date.now() - started < 2000);
  assert.equal(rowsFor(db, token).length, 0);
});

test("a take with nothing stored returns empty when its wait ends", async () => {
  const { take } = makeAuth();
  const started = Date.now();
  const r = await take(newToken(), 0.2);
  const elapsed = Date.now() - started;
  assert.equal(r.status, 200);
  assert.deepEqual(r.json, { ids: [], hello: false });
  assert.ok(elapsed >= 180, `waited ${elapsed} ms`);
  assert.ok(elapsed < 2000, `waited ${elapsed} ms`);
});

test("two takes waiting on one token never both get the same ids", async () => {
  const { ring, take } = makeAuth();
  const token = newToken();
  const ids = [newId(), newId(), newId()];
  const first = take(token, 0.5);
  const second = take(token, 0.5);
  await delay(50);
  await ring(token, ids);
  const results = await Promise.all([first, second]);
  const got = results.flatMap((r) => r.json.ids);
  assert.deepEqual(sorted(got), sorted(ids), "every id arrives exactly once");
});

test("ids rung while a take reads are kept for the next take", async () => {
  const { ring, take } = makeAuth();
  const token = newToken();
  const ids = Array.from({ length: 40 }, newId);
  const rings = ids.map((id) => ring(token, [id]));
  const takes = Array.from({ length: 5 }, () => take(token, 0));
  await Promise.all(rings);
  const taken = (await Promise.all(takes)).flatMap((r) => r.json.ids);
  const rest = (await take(token, 0)).json.ids;
  assert.deepEqual(sorted([...taken, ...rest]), sorted(ids));
});

test("requests are validated strictly", async () => {
  const { db, call, ring, take } = makeAuth();
  const token = newToken();
  const id = newId();
  const badRings = [
    { token: token.toUpperCase(), ids: [id] },
    { token: token.slice(1), ids: [id] },
    { token: `${token}0`, ids: [id] },
    { token: "g".repeat(32), ids: [id] },
    { token: [token, newToken()], ids: [id] },
    { token: 42, ids: [id] },
    { ids: [id] },
    { token, ids: [id.toUpperCase()] },
    { token, ids: [id.slice(1)] },
    { token, ids: [`${id}0`] },
    { token, ids: [123] },
    { token, ids: id },
    { token },
    { token, ids: Array.from({ length: MAX_IDS + 1 }, newId) },
  ];
  for (const body of badRings) {
    const r = await call("/commy/doorbell/ring", body);
    assert.equal(r.status, 400, `ring ${JSON.stringify(body).slice(0, 80)} answered ${r.status}`);
  }
  assert.equal((await ring(token, Array.from({ length: MAX_IDS }, newId))).status, 200, "50 ids are allowed");

  for (const body of [{ token, wait: -1 }, { token, wait: 26 }, { token, wait: "5" }, { token: "x", wait: 0 }, { wait: 0 }]) {
    const r = await call("/commy/doorbell/take", body);
    assert.equal(r.status, 400, `take ${JSON.stringify(body)} answered ${r.status}`);
  }
  assert.equal(rowsFor(db, token).length, MAX_IDS, "refused takes delete nothing");
  assert.equal((await take(token, 0)).json.ids.length, MAX_IDS);
});

test("bodies over 8 KB are refused before they are read", async () => {
  const { db, send } = makeAuth();
  const token = newToken();
  const big = JSON.stringify({ token, ids: Array.from({ length: 400 }, newId) });
  assert.ok(big.length > MAX_BODY_BYTES);
  for (const path of ["/commy/doorbell/ring", "/commy/doorbell/take"]) {
    const declared = await send(path, {
      headers: { "content-type": "application/json", "content-length": String(big.length) },
      body: big,
    });
    assert.equal(declared.status, 413, path);

    const chunked = await send(path, {
      headers: { "content-type": "application/json" },
      body: new ReadableStream({
        start(controller) {
          controller.enqueue(new TextEncoder().encode(big));
          controller.close();
        },
      }),
      duplex: "half",
    });
    assert.equal(chunked.status, 413, `${path} without a length`);
  }
  assert.equal(db.doorbellRing.length, 0);
  const padded = JSON.stringify({ token, ids: [newId()], pad: " ".repeat(7000) });
  assert.equal((await send("/commy/doorbell/ring", { headers: { "content-type": "application/json" }, body: padded })).status, 200);
});

test("a token keeps at most 1000 rows, dropping the oldest", async () => {
  const { db, ring, take } = makeAuth();
  const token = newToken();
  const other = newToken();
  const start = Date.now() - DAY_MS;
  for (let i = 0; i < MAX_PER_TOKEN; i++) {
    db.doorbellRing.push({ id: `old-${i}`, token, blindedId: newId(), createdAt: new Date(start + i * 1000) });
  }
  db.doorbellRing.push({ id: "other", token: other, blindedId: newId(), createdAt: new Date(start - 1000) });
  const fresh = [newId(), newId(), newId()];
  await ring(token, fresh);

  const rows = rowsFor(db, token);
  assert.equal(rows.length, MAX_PER_TOKEN);
  for (const gone of ["old-0", "old-1", "old-2"]) assert.ok(!rows.some((row) => row.id === gone), `${gone} was dropped`);
  assert.ok(rows.some((row) => row.id === "old-3"));
  for (const id of fresh) assert.ok(rows.some((row) => row.blindedId === id));
  assert.equal(rowsFor(db, other).length, 1, "the cap is per token");

  const taken = await take(token, 0);
  assert.equal(taken.json.ids.length, MAX_PER_TOKEN);
  assert.equal(rowsFor(db, token).length, 0);
});

async function sweptOld(db) {
  for (let i = 0; i < 100 && db.doorbellRing.some((row) => row.id.startsWith("old")); i++) await delay(10);
}

test("rows older than 14 days are swept soon after start and then on a timer", async () => {
  const now = Date.now();
  const token = newToken();
  const keep = newId();
  const rows = () => [
    { id: "old", token, blindedId: newId(), createdAt: new Date(now - MAX_AGE_MS - 60_000) },
    { id: "new", token, blindedId: keep, createdAt: new Date(now - DAY_MS) },
  ];

  // First sweep after start, before the first interval
  const first = makeAuth({ doorbell: { firstSweepMs: 5, sweepEveryMs: 60_000 } });
  first.db.doorbellRing.push(...rows());
  await sweptOld(first.db);
  assert.deepEqual(first.db.doorbellRing.map((row) => row.id), ["new"]);

  // Then every interval
  const timer = makeAuth({ doorbell: { firstSweepMs: 60_000, sweepEveryMs: 20 } });
  timer.db.doorbellRing.push(...rows());
  await sweptOld(timer.db);
  assert.deepEqual(timer.db.doorbellRing.map((row) => row.id), ["new"]);
  timer.db.doorbellRing.push({ id: "old-again", token, blindedId: newId(), createdAt: new Date(now - MAX_AGE_MS - 1) });
  await sweptOld(timer.db);
  assert.deepEqual(timer.db.doorbellRing.map((row) => row.id), ["new"]);
  assert.deepEqual((await timer.take(token, 0)).json.ids, [keep]);
});

test("a row past 14 days that the sweep has not reached is deleted, not returned", async () => {
  const { db, take } = makeAuth();
  const token = newToken();
  const keep = newId();
  db.doorbellRing.push(
    { id: "stale", token, blindedId: newId(), createdAt: new Date(Date.now() - MAX_AGE_MS - 60_000) },
    { id: "fresh", token, blindedId: keep, createdAt: new Date(Date.now() - 1000) },
  );
  assert.deepEqual((await take(token, 0)).json.ids, [keep]);
  assert.equal(db.doorbellRing.length, 0);
});

test("too many waiting takes answer 503 so apps back off", async () => {
  const { ring, take } = makeAuth({ doorbell: { maxWaiters: 1 } });
  const a = newToken();
  const b = newToken();
  const waiting = take(a, 5);
  await delay(50);
  assert.equal((await take(b, 5)).status, 503);
  assert.equal((await take(b, 0)).status, 200, "a take that does not wait still works");
  await ring(b, [newId()]);
  assert.equal((await take(b, 5)).json.ids.length, 1, "stored ids return without waiting");
  await ring(a, []);
  assert.deepEqual((await waiting).json, { ids: [], hello: true });
  assert.deepEqual((await take(b, 0.05)).json, { ids: [], hello: false }, "the slot is free again");
});

test("a token holds at most 4 waiting takes; a newer one replaces the oldest", async () => {
  const { ring, take } = makeAuth();
  const token = newToken();
  const first = take(token, 5);
  await delay(20);
  const rest = [take(token, 5), take(token, 5), take(token, 5)];
  await delay(20);
  const fifth = take(token, 5);
  await delay(50);
  assert.deepEqual((await first).json, { ids: [], hello: false }, "the oldest wait ends empty");
  const id = newId();
  await ring(token, [id]);
  const got = (await Promise.all([...rest, fifth])).flatMap((r) => r.json.ids);
  assert.deepEqual(got, [id]);
});

test("rings are refused once the table is full", async () => {
  const { db, ring, take } = makeAuth({ doorbell: { maxRows: 2 } });
  const a = newToken();
  assert.equal((await ring(a, [newId(), newId()])).status, 200);
  assert.equal((await ring(newToken(), [newId()])).status, 503);
  assert.equal((await ring(newToken(), [])).status, 200, "hellos store nothing and still go through");
  assert.equal(db.doorbellRing.length, 2);
  await take(a, 0);
  assert.equal((await ring(newToken(), [newId()])).status, 200, "takes make room");
});

test("an aborted take stops waiting", async () => {
  const { take } = makeAuth();
  const controller = new AbortController();
  const started = Date.now();
  const waiting = take(newToken(), 10, { signal: controller.signal });
  await delay(50);
  controller.abort();
  await waiting.catch(() => {});
  assert.ok(Date.now() - started < 2000);
});

test("ring and take have their own rate limits per IP", async () => {
  const options = buildAuthOptions(loadConfig(ENV), { pool: null, send: async () => {} });
  assert.deepEqual(options.rateLimit.customRules["/commy/doorbell/ring"], { window: 60, max: 300 });
  assert.deepEqual(options.rateLimit.customRules["/commy/doorbell/take"], { window: 60, max: 600 });

  const { ring, take } = makeAuth({ rateLimit: true });
  const headers = { "x-forwarded-for": "198.51.100.41" };
  const token = newToken();
  for (let i = 0; i < 600; i++) assert.equal((await take(token, 0, { headers })).status, 200, `take ${i}`);
  assert.equal((await take(token, 0, { headers })).status, 429);
  assert.equal((await take(token, 0, { headers: { "x-forwarded-for": "198.51.100.42" } })).status, 200, "other IPs are not limited");

  for (let i = 0; i < 300; i++) assert.equal((await ring(token, [], { headers })).status, 200, `ring ${i}`);
  assert.equal((await ring(token, [], { headers })).status, 429);
});
