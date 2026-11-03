import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { createSealer, sealedAtRest } from "../src/at-rest.js";
import { cleanCode, DELETE_CODE_TRIES, newDeleteCode } from "../src/account-deletion.js";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";

const ORIGIN = "http://localhost:3001";
const PASSWORD = "correct horse battery";
const SECRET = "test-secret-".repeat(4);
const MAIL = { SMTP_HOST: "smtp.invalid", SMTP_USER: "no-reply@example.test", SMTP_PASSWORD: "unused" };
const storedEmail = (row) => createSealer(SECRET).open("user", "email", row.email);
let auth;
let db;
let outbox;

function start(env) {
  const config = loadConfig({ BETTER_AUTH_SECRET: SECRET, DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN, ...env });
  outbox = [];
  const options = buildAuthOptions(config, { pool: null, send: async (mail) => outbox.push(mail) });
  db = {
    user: [],
    session: [],
    account: [],
    verification: [],
    deviceCode: [],
    linkedDevice: [],
    deviceRequest: [],
    invite: [],
    inviteBackground: [],
    directoryListing: [],
    vaultItem: [],
    doorbellRing: [],
    twoFactor: [],
    passkey: [],
  };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  auth = betterAuth(options);
  return config;
}

beforeEach(() => start(MAIL));

function cookieHeader(...setCookies) {
  const jar = new Map();
  for (const header of setCookies.filter(Boolean)) {
    for (const part of header.split(/,(?=\s*[\w.-]+=)/)) {
      const [pair, ...attrs] = part.trim().split(";");
      const [name, ...rest] = pair.split("=");
      if (attrs.some((a) => /max-age=0/i.test(a.trim()))) jar.delete(name.trim());
      else jar.set(name.trim(), rest.join("="));
    }
  }
  return [...jar].map(([k, v]) => `${k}=${v}`).join("; ");
}

async function call(method, path, { body, cookie, token } = {}) {
  const url = new URL(path.startsWith("http") ? path : `${ORIGIN}/api/auth${path}`);
  const headers = { origin: ORIGIN };
  if (body) headers["content-type"] = "application/json";
  if (cookie) headers.cookie = cookie;
  if (token) headers.authorization = `Bearer ${token}`;
  const res = await auth.handler(new Request(url, { method, headers, body: body ? JSON.stringify(body) : undefined }));
  const text = await res.text();
  let json = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    json = null;
  }
  return { status: res.status, json, setCookie: res.headers.get("set-cookie"), token: res.headers.get("set-auth-token") };
}

async function signedUp(email, { verify = true } = {}) {
  await call("POST", "/sign-up/email", { body: { email, password: PASSWORD, name: "Tester", callbackURL: "/account" } });
  if (verify) await call("GET", outbox.at(-1).url);
  const signedIn = await call("POST", "/sign-in/email", { body: { email, password: PASSWORD } });
  assert.equal(signedIn.status, 200, JSON.stringify(signedIn.json));
  outbox.length = 0;
  const user = db.user.find((u) => storedEmail(u) === email);
  return { cookie: cookieHeader(signedIn.setCookie), token: signedIn.token, user };
}

function giveThingsTo(userId, tag) {
  const now = new Date();
  const later = new Date(Date.now() + 3600_000);
  db.linkedDevice.push({ id: `dev-${tag}`, userId, nodeId: `node-${tag}`, createdAt: now, lastSeenAt: now });
  db.vaultItem.push({ id: `vault-${tag}`, userId, name: "entries", data: "sealed1.x", version: 1, updatedAt: now });
  db.passkey.push({ id: `pk-${tag}`, userId, publicKey: "k", credentialID: `cred-${tag}`, counter: 0, deviceType: "singleDevice", backedUp: false, createdAt: now });
  db.invite.push({ id: `inv-${tag}`, code: `code-${tag}`, ticket: "t", serverName: "", createdBy: userId, createdAt: now, expiresAt: later });
  db.inviteBackground.push({ id: `bg-${tag}`, code: `code-${tag}`, data: "x", expiresAt: later });
  db.directoryListing.push({ id: `dir-${tag}`, code: `code-${tag}`, key: "k", name: "n", description: "", category: "gaming", members: 10, createdBy: userId, createdAt: now, updatedAt: now });
}

const owned = (userId) =>
  ["session", "account", "linkedDevice", "vaultItem", "passkey"].flatMap((model) => db[model].filter((row) => row.userId === userId)).length +
  db.invite.filter((row) => row.createdBy === userId).length +
  db.directoryListing.filter((row) => row.createdBy === userId).length;

function codeFrom(mail) {
  return mail.body.match(/\b\d{6}\b/)[0];
}

test("an emailed code deletes the account and everything it owns, and nobody else's", async () => {
  const me = await signedUp("leaving@example.test");
  const other = await signedUp("staying@example.test");
  giveThingsTo(me.user.id, "me");
  giveThingsTo(other.user.id, "other");

  const started = await call("POST", "/commy/account/delete/start", { cookie: me.cookie });
  assert.equal(started.status, 200, JSON.stringify(started.json));
  assert.deepEqual(started.json, { confirm: "email", password: true, email: "leaving@example.test" });
  assert.equal(outbox.length, 1);
  assert.equal(outbox[0].to, "leaving@example.test");
  const code = codeFrom(outbox[0]);
  assert.ok(!db.verification.some((row) => JSON.stringify(row).includes(code)), "the code is never stored as written");

  const wrong = await call("POST", "/commy/account/delete/finish", { cookie: me.cookie, body: { code: code === "000000" ? "111111" : "000000" } });
  assert.equal(wrong.status, 400);
  assert.equal(wrong.json.code, "INVALID_CODE");
  assert.ok(db.user.some((u) => u.id === me.user.id));

  const done = await call("POST", "/commy/account/delete/finish", { cookie: me.cookie, body: { code } });
  assert.equal(done.status, 200, JSON.stringify(done.json));
  assert.deepEqual(done.json, { deleted: true });
  assert.ok(!db.user.some((u) => u.id === me.user.id));
  assert.equal(owned(me.user.id), 0);
  assert.ok(!db.inviteBackground.some((row) => row.code === "code-me"));
  assert.ok(db.user.some((u) => u.id === other.user.id));
  assert.ok(owned(other.user.id) >= 6, "the other account keeps everything");
  assert.ok(db.inviteBackground.some((row) => row.code === "code-other"));
  assert.equal(outbox.at(-1).subject, "Your Commy account was deleted");

  const session = await call("GET", "/get-session", { cookie: me.cookie });
  assert.equal(session.json, null, "the old session no longer works");
});

test("a used code cannot delete again, and too many wrong codes burn it", async () => {
  const me = await signedUp("guessing@example.test");
  await call("POST", "/commy/account/delete/start", { cookie: me.cookie });
  const code = codeFrom(outbox[0]);
  const wrong = code === "000000" ? "111111" : "000000";
  for (let i = 0; i < DELETE_CODE_TRIES; i++) {
    const r = await call("POST", "/commy/account/delete/finish", { cookie: me.cookie, body: { code: wrong } });
    assert.equal(r.json.code, "INVALID_CODE");
  }
  const late = await call("POST", "/commy/account/delete/finish", { cookie: me.cookie, body: { code } });
  assert.equal(late.status, 400, "the right code no longer works once it has been guessed at too often");
  assert.ok(db.user.some((u) => u.id === me.user.id));

  await call("POST", "/commy/account/delete/start", { cookie: me.cookie });
  const fresh = codeFrom(outbox.at(-1));
  const ok = await call("POST", "/commy/account/delete/finish", { cookie: me.cookie, body: { code: fresh } });
  assert.equal(ok.status, 200);
});

test("asking again replaces the earlier code", async () => {
  const me = await signedUp("twice@example.test");
  await call("POST", "/commy/account/delete/start", { cookie: me.cookie });
  const first = codeFrom(outbox[0]);
  await call("POST", "/commy/account/delete/start", { cookie: me.cookie });
  const second = codeFrom(outbox[1]);
  if (first !== second) {
    const r = await call("POST", "/commy/account/delete/finish", { cookie: me.cookie, body: { code: first } });
    assert.equal(r.status, 400);
  }
  const ok = await call("POST", "/commy/account/delete/finish", { cookie: me.cookie, body: { code: second } });
  assert.equal(ok.status, 200);
});

test("the account password confirms without a code, a wrong one does not", async () => {
  const me = await signedUp("password@example.test");
  const wrong = await call("POST", "/commy/account/delete/finish", { cookie: me.cookie, body: { password: "not the password" } });
  assert.equal(wrong.status, 400);
  assert.equal(wrong.json.code, "INVALID_PASSWORD");
  const ok = await call("POST", "/commy/account/delete/finish", { cookie: me.cookie, body: { password: PASSWORD } });
  assert.equal(ok.status, 200);
  assert.ok(!db.user.some((u) => u.id === me.user.id));
});

test("a signed in session alone cannot delete an account", async () => {
  const me = await signedUp("nothing@example.test");
  const r = await call("POST", "/commy/account/delete/finish", { cookie: me.cookie, body: {} });
  assert.equal(r.status, 400);
  assert.equal(r.json.code, "CONFIRMATION_REQUIRED");
  const anon = await call("POST", "/commy/account/delete/finish", { body: { password: PASSWORD } });
  assert.equal(anon.status, 401);
  const anonStart = await call("POST", "/commy/account/delete/start", {});
  assert.equal(anonStart.status, 401);
  assert.equal(outbox.length, 0);
});

test("apps signed in with a bearer token can delete their account", async () => {
  const me = await signedUp("phone@example.test");
  assert.ok(me.token);
  await call("POST", "/commy/account/delete/start", { token: me.token });
  const ok = await call("POST", "/commy/account/delete/finish", { token: me.token, body: { code: codeFrom(outbox[0]) } });
  assert.equal(ok.status, 200, JSON.stringify(ok.json));
  assert.ok(!db.user.some((u) => u.id === me.user.id));
});

test("without email a fresh sign-in confirms", async () => {
  start({});
  const me = await signedUp("selfhost@example.test", { verify: false });
  const started = await call("POST", "/commy/account/delete/start", { cookie: me.cookie });
  assert.deepEqual(started.json, { confirm: "password", password: true, email: null });
  const ok = await call("POST", "/commy/account/delete/finish", { cookie: me.cookie, body: {} });
  assert.equal(ok.status, 200, JSON.stringify(ok.json));
  assert.ok(!db.user.some((u) => u.id === me.user.id));
});

test("codes are six digits and typed spacing is forgiven", () => {
  for (let i = 0; i < 50; i++) assert.match(newDeleteCode(), /^\d{6}$/);
  assert.equal(cleanCode("123 456"), "123456");
  assert.equal(cleanCode("123-456"), "123456");
  assert.equal(cleanCode("12345"), null);
  assert.equal(cleanCode("abcdef"), null);
  assert.equal(cleanCode(undefined), null);
});

test("the discovery document names the deletion endpoints", () => {
  const doc = discoveryDocument(start(MAIL));
  assert.equal(doc.endpoints.accountDeleteStart, `${ORIGIN}/api/auth/commy/account/delete/start`);
  assert.equal(doc.endpoints.accountDeleteFinish, `${ORIGIN}/api/auth/commy/account/delete/finish`);
});
