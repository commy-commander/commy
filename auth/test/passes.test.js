import assert from "node:assert/strict";
import { createPublicKey, randomBytes } from "node:crypto";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { isSealed, sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";
import {
  blindedMessage,
  createPassBuckets,
  epochOf,
  MAX_PASSES_PER_REQUEST,
  PASS_BURST,
  PASS_KEY_BYTES,
  PASS_REFILL_MS,
  refill,
  storedTokens,
} from "../src/passes.js";

const ORIGIN = "http://localhost:3001";
const PASSWORD = "correct horse battery";
const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN, COMMY_ABUSE_CONTACT: "safety@example.org" };

function start() {
  const options = buildAuthOptions(loadConfig(ENV), { pool: null, send: async () => {} });
  const db = {
    user: [], session: [], commyIdentity: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [],
    invite: [], inviteBackground: [], doorbellRing: [], directoryListing: [], commyMove: [], commyPassKey: [], commyPassBucket: [],
  };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  const auth = betterAuth(options);
  const call = async (method, path, { body, cookie } = {}) => {
    const headers = { origin: ORIGIN, cookie: cookie ?? "x=1" };
    if (body) headers["content-type"] = "application/json";
    const res = await auth.handler(new Request(`${ORIGIN}/api/auth${path}`, { method, headers, body: body ? JSON.stringify(body) : undefined }));
    const text = await res.text();
    return { status: res.status, json: text ? JSON.parse(text) : null, headers: res.headers, setCookie: res.headers.get("set-cookie") };
  };
  const signUp = async (email) => {
    const r = await call("POST", "/sign-up/email", { body: { email, password: PASSWORD, name: "T" } });
    assert.equal(r.status, 200, JSON.stringify(r.json));
    return { cookie: r.setCookie.split(";")[0], id: r.json.user.id };
  };
  return { db, call, signUp };
}

const bigOf = (bytes) => BigInt(`0x${Buffer.from(bytes).toString("hex") || "0"}`);

function modPow(base, exponent, modulus) {
  let result = 1n;
  let b = base % modulus;
  let e = exponent;
  while (e > 0n) {
    if (e & 1n) result = (result * b) % modulus;
    b = (b * b) % modulus;
    e >>= 1n;
  }
  return result;
}

function publicNumbers(der) {
  const jwk = createPublicKey({ key: Buffer.from(der, "base64url"), format: "der", type: "spki" }).export({ format: "jwk" });
  return { n: bigOf(Buffer.from(jwk.n, "base64url")), e: bigOf(Buffer.from(jwk.e, "base64url")) };
}

function blindedBelow(n) {
  for (;;) {
    const bytes = randomBytes(PASS_KEY_BYTES);
    if (bigOf(bytes) < n) return bytes;
  }
}

async function weekKey(world) {
  const r = await world.call("GET", "/commy/passes/keys");
  assert.equal(r.status, 200, JSON.stringify(r.json));
  const epoch = epochOf(Date.now());
  const key = r.json.keys.find((k) => k.epoch === epoch);
  assert.ok(key, "this week's key is published");
  return { epoch, ...publicNumbers(key.der) };
}

test("weeks count from the epoch the same way sync counts them", () => {
  const week = 7 * 24 * 60 * 60 * 1000;
  assert.equal(epochOf(0), 0);
  assert.equal(epochOf(week - 1), 0);
  assert.equal(epochOf(week), 1);
  assert.equal(epochOf(-5), 0);
});

test("blinded passes must be exactly one key long", () => {
  assert.equal(blindedMessage(Buffer.alloc(PASS_KEY_BYTES, 1).toString("base64url")).length, PASS_KEY_BYTES);
  for (const bad of [Buffer.alloc(PASS_KEY_BYTES - 1, 1).toString("base64url"), "not base64!", "", null, 5]) assert.equal(blindedMessage(bad), null);
});

test("each account gets a burst of passes, then one more at a slow rate", async () => {
  let now = 1_000_000;
  const rows = [];
  const adapter = {
    async findOne({ where }) {
      return rows.find((row) => row.userId === where[0].value) ?? null;
    },
    async create({ data }) {
      rows.push({ id: String(rows.length + 1), ...data });
    },
    async update({ where, update }) {
      Object.assign(rows.find((row) => row.id === where[0].value), update);
    },
  };
  const buckets = createPassBuckets({ now: () => now });
  const taken = await Promise.all(Array.from({ length: 5 }, () => buckets.take(adapter, "alice", MAX_PASSES_PER_REQUEST)));
  assert.deepEqual(taken.map((t) => t.granted), [10, 10, 10, 0, 0], "requests made at once still share one bucket");
  assert.ok(taken[3].waitMs > 0 && taken[3].waitMs <= PASS_REFILL_MS);
  assert.equal((await buckets.take(adapter, "bob", 3)).granted, 3, "accounts have their own buckets");
  now += PASS_REFILL_MS;
  assert.equal((await buckets.take(adapter, "alice", MAX_PASSES_PER_REQUEST)).granted, 1);
  now += 1000 * PASS_REFILL_MS;
  assert.equal((await buckets.take(adapter, "alice", 100)).granted, PASS_BURST, "a long rest fills it only to the burst");
  assert.equal(refill(null, now), PASS_BURST);
});

test("a part-filled bucket is stored as whole numbers and read back to the thousandth", async () => {
  let now = 5_000_000;
  const rows = [];
  const adapter = {
    async findOne({ where }) {
      return rows.find((row) => row.userId === where[0].value) ?? null;
    },
    async create({ data }) {
      rows.push({ id: "1", ...data });
    },
    async update({ where, update }) {
      Object.assign(rows.find((row) => row.id === where[0].value), update);
    },
  };
  const buckets = createPassBuckets({ now: () => now });
  await buckets.take(adapter, "alice", MAX_PASSES_PER_REQUEST);
  now += Math.round(0.3123 * PASS_REFILL_MS);
  await buckets.take(adapter, "alice", MAX_PASSES_PER_REQUEST);
  const [row] = rows;
  assert.ok(Number.isInteger(row.tokens) && Number.isInteger(row.milliTokens), JSON.stringify(row));
  assert.equal(row.milliTokens, 10312);
  assert.equal(row.tokens, 10);
  assert.ok(Math.abs(storedTokens(row) - 10.312) < 1e-9);
  assert.equal(storedTokens({ tokens: 7 }), 7, "rows written before milli-tokens still read");
});

test("the key list is public and holds this week and next week", async () => {
  const world = start();
  const r = await world.call("GET", "/commy/passes/keys");
  assert.equal(r.status, 200);
  assert.match(r.headers.get("cache-control"), /public/);
  const epoch = epochOf(Date.now());
  assert.deepEqual(r.json.keys.map((k) => k.epoch), [epoch, epoch + 1]);
  for (const key of r.json.keys) {
    const { n, e } = publicNumbers(key.der);
    assert.equal(n.toString(2).length, 2048);
    assert.equal(e, 65537n);
  }
  assert.deepEqual((await world.call("GET", "/commy/passes/keys")).json, r.json, "keys stay put within a week");
  assert.equal(world.db.commyPassKey.length, 2);
  assert.ok(world.db.commyPassKey.every((row) => isSealed(row.privateKey)), "private halves are encrypted at rest");
  const d = discoveryDocument(loadConfig(ENV));
  assert.equal(d.endpoints.passKeys, `${ORIGIN}/api/auth/commy/passes/keys`);
  assert.equal(d.endpoints.passes, `${ORIGIN}/api/auth/commy/passes`);
});

test("a signed-in account gets blind signatures that check against the week's key", async () => {
  const world = start();
  const { n, e, epoch } = await weekKey(world);
  assert.equal((await world.call("POST", "/commy/passes", { body: { epoch, blinded: [blindedBelow(n).toString("base64url")] } })).status, 401);
  const alice = await world.signUp("alice@example.test");
  const blinded = Array.from({ length: 3 }, () => blindedBelow(n));
  const r = await world.call("POST", "/commy/passes", { cookie: alice.cookie, body: { epoch, blinded: blinded.map((b) => b.toString("base64url")) } });
  assert.equal(r.status, 200, JSON.stringify(r.json));
  assert.equal(r.json.epoch, epoch);
  assert.equal(r.json.signatures.length, 3);
  r.json.signatures.forEach((sig, i) => {
    const bytes = Buffer.from(sig, "base64url");
    assert.equal(bytes.length, PASS_KEY_BYTES);
    assert.equal(modPow(bigOf(bytes), e, n), bigOf(blinded[i]), "each signature opens back to what was blinded");
  });
  assert.equal(JSON.stringify(world.db.commyPassBucket).includes(blinded[0].toString("base64url")), false, "nothing blinded is kept");
});

test("passes are refused for other weeks, bad sizes, and once the bucket is empty", async () => {
  const world = start();
  const { n, epoch } = await weekKey(world);
  const alice = await world.signUp("alice@example.test");
  const ask = (body) => world.call("POST", "/commy/passes", { cookie: alice.cookie, body });
  const some = (count) => Array.from({ length: count }, () => blindedBelow(n).toString("base64url"));
  assert.equal((await ask({ epoch: epoch + 1, blinded: some(1) })).json.code, "WRONG_WEEK");
  assert.equal((await ask({ epoch, blinded: ["AAAA"] })).status, 400);
  assert.equal((await ask({ epoch, blinded: [Buffer.alloc(PASS_KEY_BYTES, 0xff).toString("base64url")] })).status, 400, "a value past the modulus is refused");
  assert.equal((await ask({ epoch, blinded: some(MAX_PASSES_PER_REQUEST + 1) })).status, 400);
  assert.equal(world.db.commyPassBucket.length, 0, "refused requests cost nothing");
  let got = 0;
  for (let i = 0; i < 3; i++) got += (await ask({ epoch, blinded: some(MAX_PASSES_PER_REQUEST) })).json.signatures.length;
  assert.equal(got, PASS_BURST);
  const empty = await ask({ epoch, blinded: some(1) });
  assert.equal(empty.status, 429);
  assert.equal(empty.json.code, "NO_PASSES_LEFT");
  assert.ok(Number(empty.headers.get("retry-after")) >= 1);
});

test("a suspended account gets no passes", async () => {
  const world = start();
  const { n, epoch } = await weekKey(world);
  const mallory = await world.signUp("mallory@example.test");
  world.db.user.find((u) => u.id === mallory.id).suspendedAt = new Date();
  const r = await world.call("POST", "/commy/passes", { cookie: mallory.cookie, body: { epoch, blinded: [blindedBelow(n).toString("base64url")] } });
  assert.equal(r.status, 403);
  assert.equal(r.json.code, "ACCOUNT_SUSPENDED");
});
