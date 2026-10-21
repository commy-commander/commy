import assert from "node:assert/strict";
import { generateKeyPairSync, sign } from "node:crypto";
import { after, before, test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { getMigrations } from "better-auth/db/migration";
import pg from "pg";
import { SEALED_FIELDS, createSealer, isSealed, openEverySealedRow, sealAdapter, sealExistingRows, sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { loadConfig } from "../src/config.js";

const ORIGIN = "http://localhost:3001";
const SECRET = "at-rest-secret-".repeat(3);
const EMAIL = "dana@example.test";
const NAME = "Dana Tester";
const PASSWORD = "correct horse battery";

function config(extra = {}) {
  return loadConfig({
    BETTER_AUTH_SECRET: SECRET,
    DATABASE_URL: "postgres://unused",
    BETTER_AUTH_URL: ORIGIN,
    SMTP_HOST: "smtp.invalid",
    SMTP_USER: "no-reply@example.test",
    SMTP_PASSWORD: "unused",
    ...extra,
  });
}

function nodeKey() {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const nodeId = publicKey.export({ format: "der", type: "spki" }).subarray(12).toString("hex");
  return { nodeId, sign: (message) => sign(null, Buffer.from(message), privateKey).toString("hex") };
}

function client(auth) {
  return async function call(method, path, { body, cookie, token, query } = {}) {
    const url = new URL(path.startsWith("http") ? path : `${ORIGIN}/api/auth${path}`);
    for (const [k, v] of Object.entries(query ?? {})) url.searchParams.set(k, v);
    const headers = { origin: ORIGIN };
    if (body) headers["content-type"] = "application/json";
    if (cookie) headers.cookie = cookie;
    if (token) headers.authorization = `Bearer ${token}`;
    const res = await auth.handler(new Request(url, { method, headers, body: body ? JSON.stringify(body) : undefined, redirect: "manual" }));
    const text = await res.text();
    let json = null;
    try {
      json = text ? JSON.parse(text) : null;
    } catch {}
    return { status: res.status, json, setCookie: res.headers.get("set-cookie") ?? "", location: res.headers.get("location") };
  };
}

const sessionCookie = (setCookie) => setCookie.split(/,(?=\s*[\w.-]+=)/).map((c) => c.split(";")[0].trim()).find((c) => c.includes("session_token"));

test("lookup fields seal the same way every time, private fields never do", () => {
  const sealer = createSealer(SECRET);
  const a = sealer.seal("user", "email", EMAIL);
  assert.equal(a, sealer.seal("user", "email", EMAIL));
  assert.ok(a.startsWith("det1.") && !a.includes("dana"));
  assert.notEqual(sealer.seal("user", "name", NAME), sealer.seal("user", "name", NAME));
  assert.equal(sealer.open("user", "email", a), EMAIL);
  assert.equal(sealer.open("user", "name", sealer.seal("user", "name", NAME)), NAME);
});

test("a sealed value only opens in its own column with the same secret", () => {
  const sealer = createSealer(SECRET);
  const token = sealer.seal("session", "token", "abc");
  assert.throws(() => sealer.open("user", "email", token));
  assert.throws(() => createSealer("another-secret-".repeat(3)).open("session", "token", token));
  assert.notEqual(sealer.seal("session", "token", "abc"), sealer.seal("deviceCode", "deviceCode", "abc"));
});

test("values that look sealed are still sealed on the way in", () => {
  const sealer = createSealer(SECRET);
  const lookalike = "enc1.not-really";
  const stored = sealer.seal("user", "name", lookalike);
  assert.notEqual(stored, lookalike);
  assert.equal(sealer.open("user", "name", stored), lookalike);
  assert.equal(sealer.open("user", "name", "plain from before"), "plain from before");
  assert.equal(sealer.seal("invite", "code", "ABCDEFGH"), "ABCDEFGH");
});

test("sealed columns cannot be searched, sorted or matched loosely", async () => {
  const db = { user: [] };
  const adapter = sealAdapter(memoryAdapter(db)({}), createSealer(SECRET));
  await assert.rejects(adapter.findMany({ model: "user", where: [{ field: "email", operator: "contains", value: "dana" }] }), /matched exactly/);
  await assert.rejects(adapter.findMany({ model: "user", where: [{ field: "email", value: EMAIL, mode: "insensitive" }] }), /matched exactly/);
  await assert.rejects(adapter.findMany({ model: "user", where: [{ field: "name", value: NAME }] }), /cannot be searched/);
  await assert.rejects(adapter.findMany({ model: "user", sortBy: { field: "name", direction: "asc" } }), /cannot be sorted/);
});

function memoryAuth(db) {
  const options = buildAuthOptions(config(), { pool: null, send: async (mail) => outbox.push(mail) });
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  return betterAuth(options);
}

let outbox = [];

test("nothing readable about an account is left in the stored rows", { timeout: 30000 }, async () => {
  outbox = [];
  const db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], inviteBackground: [], doorbellRing: [], twoFactor: [], passkey: [] };
  const call = client(memoryAuth(db));

  const signUp = await call("POST", "/sign-up/email", { body: { email: EMAIL, password: PASSWORD, name: NAME } });
  assert.equal(signUp.status, 200, JSON.stringify(signUp.json));
  const verify = await call("GET", outbox.at(-1).url);
  assert.ok([200, 302].includes(verify.status), JSON.stringify(verify.json));
  const cookie = sessionCookie((await call("POST", "/sign-in/email", { body: { email: EMAIL, password: PASSWORD } })).setCookie);
  assert.ok(cookie);

  const magic = await call("POST", "/sign-in/magic-link", { body: { email: EMAIL, callbackURL: "/account" } });
  assert.equal(magic.status, 200, JSON.stringify(magic.json));
  const magicToken = new URL(outbox.at(-1).url).searchParams.get("token");
  const reset = await call("POST", "/request-password-reset", { body: { email: EMAIL, redirectTo: "/reset" } });
  assert.equal(reset.status, 200, JSON.stringify(reset.json));

  const code = await call("POST", "/device/code", { body: { client_id: "commy-desktop" } });
  await call("GET", "/device", { cookie, query: { user_code: code.json.user_code } });
  const approve = await call("POST", "/device/approve", { cookie, body: { userCode: code.json.user_code } });
  assert.equal(approve.status, 200, JSON.stringify(approve.json));
  await new Promise((r) => setTimeout(r, 5100));
  const granted = await call("POST", "/device/token", {
    body: { grant_type: "urn:ietf:params:oauth:grant-type:device_code", device_code: code.json.device_code, client_id: "commy-desktop" },
  });
  assert.equal(granted.status, 200, JSON.stringify(granted.json));
  const token = granted.json.access_token;

  const key = nodeKey();
  const ch = await call("POST", "/commy/devices/challenge", { token, body: { nodeId: key.nodeId } });
  const linked = await call("POST", "/commy/devices/register", {
    token,
    body: { nodeId: key.nodeId, challenge: ch.json.challenge, signature: key.sign(ch.json.message) },
  });
  assert.equal(linked.status, 200, JSON.stringify(linked.json));
  assert.equal(linked.json.device.nodeId, key.nodeId);

  const me = await call("GET", "/get-session", { token });
  assert.equal(me.json.user.email, EMAIL);
  assert.equal(me.json.user.name, NAME);
  assert.equal((await call("GET", "/commy/devices", { cookie })).json.devices[0].nodeId, key.nodeId);

  const dump = JSON.stringify(db);
  const secrets = [EMAIL, NAME, key.nodeId, token, magicToken, code.json.device_code, cookie.split("=")[1].split(".")[0]];
  for (const secret of secrets) assert.ok(!dump.includes(secret), `${secret} is readable in the stored rows`);
  for (const [model, fields] of Object.entries(SEALED_FIELDS)) {
    for (const row of db[model] ?? []) {
      for (const field of Object.keys(fields)) if (typeof row[field] === "string") assert.ok(isSealed(row[field]), `${model}.${field}`);
    }
  }

  const storedToken = db.session.find((s) => createSealer(SECRET).open("session", "token", s.token) === token).token;
  assert.equal((await call("GET", "/get-session", { token: storedToken })).json, null, "a stored token is not a session");

  const sessions = await call("GET", "/list-sessions", { cookie });
  assert.ok(sessions.json.some((s) => s.token === token));
  const revoked = await call("POST", "/revoke-session", { cookie, body: { token } });
  assert.equal(revoked.status, 200, JSON.stringify(revoked.json));
  assert.equal((await call("GET", "/get-session", { token })).json, null);

  const magicSignIn = await call("GET", "/magic-link/verify", { query: { token: magicToken, callbackURL: "/account" } });
  assert.equal(magicSignIn.status, 302);
  assert.ok(sessionCookie(magicSignIn.setCookie), "a hashed magic link still signs in");
});

const POSTGRES = process.env.POSTGRES_TEST_URL;
let admin;
let pool;

before(async () => {
  if (!POSTGRES) return;
  admin = new pg.Pool({ connectionString: POSTGRES, max: 1 });
  const name = `commy_at_rest_${process.pid}`;
  await admin.query(`DROP DATABASE IF EXISTS ${name}`);
  await admin.query(`CREATE DATABASE ${name}`);
  const url = new URL(POSTGRES);
  url.pathname = `/${name}`;
  pool = new pg.Pool({ connectionString: url.toString(), max: 5 });
});

after(async () => {
  await pool?.end();
  await admin?.end();
});

function postgresAuth({ sealed }) {
  const options = buildAuthOptions(config(), { pool, send: async (mail) => outbox.push(mail) });
  if (!sealed) options.database = pool;
  options.rateLimit = { enabled: false };
  return { auth: betterAuth(options), options };
}

test("rows written before this change are sealed at startup and can be opened again", { skip: !POSTGRES && "no Postgres: set POSTGRES_TEST_URL" }, async () => {
  outbox = [];
  const before = postgresAuth({ sealed: false });
  const { runMigrations } = await getMigrations({ ...before.options, database: pool });
  await runMigrations();
  const oldCall = client(before.auth);
  assert.equal((await oldCall("POST", "/sign-up/email", { body: { email: EMAIL, password: PASSWORD, name: NAME } })).status, 200);
  await oldCall("GET", outbox.at(-1).url);
  const oldCookie = sessionCookie((await oldCall("POST", "/sign-in/email", { body: { email: EMAIL, password: PASSWORD } })).setCookie);
  assert.equal((await pool.query('SELECT email FROM "user"')).rows[0].email, EMAIL);

  const overlapping = client(postgresAuth({ sealed: true }).auth);
  assert.equal((await overlapping("GET", "/get-session", { cookie: oldCookie })).json.user.email, EMAIL, "a new server reads rows an older one wrote");
  const signedInDuringOverlap = sessionCookie((await overlapping("POST", "/sign-in/email", { body: { email: EMAIL, password: PASSWORD } })).setCookie);
  assert.ok(signedInDuringOverlap);

  const sealedCount = await sealExistingRows(pool, SECRET);
  assert.ok(sealedCount >= 4, `sealed ${sealedCount}`);
  assert.equal(await sealExistingRows(pool, SECRET), 0, "sealing twice changes nothing");
  const raw = JSON.stringify((await pool.query('SELECT u.email, u.name, s.token FROM "user" u JOIN "session" s ON s."userId" = u.id')).rows);
  assert.ok(!raw.includes(EMAIL) && !raw.includes(NAME));

  const call = client(postgresAuth({ sealed: true }).auth);
  assert.equal((await call("GET", "/get-session", { cookie: oldCookie })).json.user.email, EMAIL, "existing sessions survive");
  assert.equal((await call("GET", "/get-session", { cookie: signedInDuringOverlap })).json.user.email, EMAIL);
  const fresh = await call("POST", "/sign-in/email", { body: { email: EMAIL, password: PASSWORD } });
  assert.equal(fresh.status, 200, JSON.stringify(fresh.json));
  await call("POST", "/sign-up/email", { body: { email: EMAIL, password: PASSWORD, name: "Someone" } });
  assert.equal((await pool.query('SELECT count(*)::int AS n FROM "user"')).rows[0].n, 1, "the same email cannot sign up twice");

  assert.ok((await openEverySealedRow(pool, SECRET)) >= 4);
  assert.equal((await pool.query('SELECT email FROM "user"')).rows[0].email, EMAIL);
  const rolledBack = await oldCall("POST", "/sign-in/email", { body: { email: EMAIL, password: PASSWORD } });
  assert.equal(rolledBack.status, 200, "an older server works again after opening the rows");
});
