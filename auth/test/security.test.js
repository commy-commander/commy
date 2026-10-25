import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";
import { createHmac } from "node:crypto";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { createSealer, sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { loadConfig } from "../src/config.js";
import { noticeWhenALoginIsLinked, safeReturnPath } from "../src/security.js";

const ORIGIN = "http://localhost:3001";
const PASSWORD = "correct horse battery";
const SECRET = "test-secret-".repeat(4);
const storedEmail = (row) => createSealer(SECRET).open("user", "email", row.email);
let auth;
let db;
let outbox;

beforeEach(() => {
  const config = loadConfig({
    BETTER_AUTH_SECRET: SECRET,
    DATABASE_URL: "postgres://unused",
    BETTER_AUTH_URL: ORIGIN,
    SMTP_HOST: "smtp.invalid",
    SMTP_USER: "no-reply@example.test",
    SMTP_PASSWORD: "unused",
  });
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
    invite: [], inviteBackground: [],
    doorbellRing: [],
    twoFactor: [],
    passkey: [],
  };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  auth = betterAuth(options);
});

function cookieHeader(...setCookies) {
  const jar = new Map();
  for (const header of setCookies.filter(Boolean)) {
    for (const part of header.split(/,(?=\s*[\w.-]+=)/)) {
      const [pair, ...attrs] = part.trim().split(";");
      const [name, ...rest] = pair.split("=");
      const expired = attrs.some((a) => /max-age=0/i.test(a.trim()));
      if (expired) jar.delete(name.trim());
      else jar.set(name.trim(), rest.join("="));
    }
  }
  return [...jar].map(([k, v]) => `${k}=${v}`).join("; ");
}

async function call(method, path, { body, cookie } = {}) {
  const url = new URL(path.startsWith("http") ? path : `${ORIGIN}/api/auth${path}`);
  const headers = { origin: ORIGIN };
  if (body) headers["content-type"] = "application/json";
  if (cookie) headers.cookie = cookie;
  const res = await auth.handler(new Request(url, { method, headers, body: body ? JSON.stringify(body) : undefined }));
  const text = await res.text();
  let json = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    json = null;
  }
  return { status: res.status, json, location: res.headers.get("location"), setCookie: res.headers.get("set-cookie") };
}

async function verifiedAccount(email) {
  await call("POST", "/sign-up/email", { body: { email, password: PASSWORD, name: "Tester", callbackURL: "/account" } });
  await call("GET", outbox.at(-1).url);
  const signedIn = await call("POST", "/sign-in/email", { body: { email, password: PASSWORD } });
  assert.equal(signedIn.status, 200, JSON.stringify(signedIn.json));
  outbox.length = 0;
  return cookieHeader(signedIn.setCookie);
}

async function magicLinkAccount(email) {
  await call("POST", "/sign-in/magic-link", { body: { email, callbackURL: "/account" } });
  const verify = await call("GET", outbox.at(-1).url);
  assert.equal(verify.status, 302);
  outbox.length = 0;
  return cookieHeader(verify.setCookie);
}

async function turnOnTwoFactor(cookie, password) {
  const enabled = await call("POST", "/two-factor/enable", { cookie, body: password ? { password } : {} });
  assert.equal(enabled.status, 200, JSON.stringify(enabled.json));
  assert.equal(enabled.json.backupCodes.length, 10);
  const confirm = await call("POST", "/two-factor/verify-totp", { cookie, body: { code: await totpFor(enabled.json.totpURI) } });
  assert.equal(confirm.status, 200, JSON.stringify(confirm.json));
  return { totpURI: enabled.json.totpURI, backupCodes: enabled.json.backupCodes, cookie: cookieHeader(cookie, confirm.setCookie) };
}

const BASE32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

function fromBase32(text) {
  let bits = "";
  for (const c of text.replace(/=+$/, "").toUpperCase()) bits += BASE32.indexOf(c).toString(2).padStart(5, "0");
  const bytes = [];
  for (let i = 0; i + 8 <= bits.length; i += 8) bytes.push(parseInt(bits.slice(i, i + 8), 2));
  return Buffer.from(bytes);
}

async function totpFor(totpURI) {
  const key = fromBase32(new URL(totpURI).searchParams.get("secret"));
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(Math.floor(Date.now() / 30000)));
  const mac = createHmac("sha1", key).update(counter).digest();
  const offset = mac[mac.length - 1] & 15;
  return String((mac.readUInt32BE(offset) & 0x7fffffff) % 1_000_000).padStart(6, "0");
}

test("the last way to sign in cannot be unlinked", async () => {
  const cookie = await verifiedAccount("solo@example.test");
  const accounts = await call("GET", "/list-accounts", { cookie });
  assert.deepEqual(accounts.json.map((a) => a.providerId), ["credential"]);
  const r = await call("POST", "/unlink-account", { cookie, body: { accountId: accounts.json[0].id } });
  assert.equal(r.status, 400);
  assert.equal(r.json.code, "FAILED_TO_UNLINK_LAST_ACCOUNT");
  assert.equal(db.account.length, 1);
  assert.equal(outbox.length, 0, "no notice for a refused change");
});

test("unlinking a social login keeps the password and sends a notice", async () => {
  const cookie = await verifiedAccount("both@example.test");
  const user = db.user.find((u) => storedEmail(u) === "both@example.test");
  db.account.push({ id: "acc-google", userId: user.id, providerId: "google", accountId: "g-1", createdAt: new Date(), updatedAt: new Date() });
  const google = (await call("GET", "/list-accounts", { cookie })).json.find((a) => a.providerId === "google");
  const r = await call("POST", "/unlink-account", { cookie, body: { accountId: google.id } });
  assert.equal(r.status, 200, JSON.stringify(r.json));
  assert.deepEqual(db.account.map((a) => a.providerId), ["credential"]);
  assert.equal(outbox.length, 1);
  assert.equal(outbox[0].to, "both@example.test");
  assert.match(outbox[0].subject, /removed/);
});

test("an account without a password can set one once", async () => {
  const cookie = await magicLinkAccount("nopass@example.test");
  assert.equal((await call("POST", "/sign-in/email", { body: { email: "nopass@example.test", password: PASSWORD } })).status, 401);

  const short = await call("POST", "/commy/password/set", { cookie, body: { newPassword: "short" } });
  assert.equal(short.status, 400);
  const set = await call("POST", "/commy/password/set", { cookie, body: { newPassword: PASSWORD } });
  assert.equal(set.status, 200, JSON.stringify(set.json));
  assert.equal((await call("POST", "/sign-in/email", { body: { email: "nopass@example.test", password: PASSWORD } })).status, 200);
  assert.match(outbox.at(-1).subject, /password was added/);

  const again = await call("POST", "/commy/password/set", { cookie, body: { newPassword: "a different passphrase" } });
  assert.equal(again.status, 400);
  assert.equal(again.json.code, "PASSWORD_ALREADY_SET");
  assert.equal((await call("POST", "/commy/password/set", { body: { newPassword: PASSWORD } })).status, 401);
});

test("changing the password needs the old one and sends a notice", async () => {
  const cookie = await verifiedAccount("change@example.test");
  const wrong = await call("POST", "/change-password", { cookie, body: { currentPassword: "nope nope nope", newPassword: "brand new passphrase" } });
  assert.equal(wrong.status, 400);
  assert.equal(outbox.length, 0);
  const ok = await call("POST", "/change-password", {
    cookie,
    body: { currentPassword: PASSWORD, newPassword: "brand new passphrase", revokeOtherSessions: true },
  });
  assert.equal(ok.status, 200, JSON.stringify(ok.json));
  assert.equal(outbox.length, 1);
  assert.match(outbox[0].subject, /password was changed/);
  assert.equal(outbox[0].url, `${ORIGIN}/account`);
});

test("changing email is confirmed from the old inbox, then the new one", async () => {
  const cookie = await verifiedAccount("old@example.test");
  const r = await call("POST", "/change-email", { cookie, body: { newEmail: "new@example.test", callbackURL: "/account" } });
  assert.equal(r.status, 200, JSON.stringify(r.json));
  assert.equal(outbox.length, 1);
  assert.equal(outbox[0].to, "old@example.test");
  assert.match(outbox[0].body, /new@example\.test/);
  assert.equal(storedEmail(db.user[0]), "old@example.test");

  const confirmed = await call("GET", outbox[0].url, { cookie });
  assert.equal(confirmed.status, 302);
  assert.equal(confirmed.location, "/account");
  assert.equal(outbox.length, 2);
  assert.equal(outbox[1].to, "new@example.test");
  assert.equal(storedEmail(db.user[0]), "old@example.test", "nothing changes until the new address answers");

  const verified = await call("GET", outbox[1].url, { cookie });
  assert.equal(verified.status, 302);
  assert.equal(verified.location, "/account");
  assert.equal(storedEmail(db.user[0]), "new@example.test");
  assert.equal(db.user[0].emailVerified, true);
});

test("two-step sign-in asks for a code after the password", async () => {
  const cookie = await verifiedAccount("totp@example.test");
  const { totpURI, backupCodes } = await turnOnTwoFactor(cookie, PASSWORD);
  assert.match(totpURI, /^otpauth:\/\/totp\/Commy:totp%40example\.test\?/);
  assert.equal(db.user[0].twoFactorEnabled, true);

  const first = await call("POST", "/sign-in/email", { body: { email: "totp@example.test", password: PASSWORD } });
  assert.equal(first.status, 200);
  assert.equal(first.json.twoFactorRedirect, true);
  assert.doesNotMatch(cookieHeader(first.setCookie), /session_token=/);
  const challenge = cookieHeader(first.setCookie);

  const wrong = await call("POST", "/two-factor/verify-totp", { cookie: challenge, body: { code: "000000" } });
  assert.equal(wrong.status, 401);
  const right = await call("POST", "/two-factor/verify-totp", { cookie: challenge, body: { code: await totpFor(totpURI) } });
  assert.equal(right.status, 200, JSON.stringify(right.json));
  assert.match(cookieHeader(right.setCookie), /session_token=/);

  const again = await call("POST", "/sign-in/email", { body: { email: "totp@example.test", password: PASSWORD } });
  const backup = await call("POST", "/two-factor/verify-backup-code", { cookie: cookieHeader(again.setCookie), body: { code: backupCodes[0] } });
  assert.equal(backup.status, 200, JSON.stringify(backup.json));
});

test("a magic link does not skip two-step sign-in", async () => {
  const cookie = await magicLinkAccount("linkonly@example.test");
  const { totpURI } = await turnOnTwoFactor(cookie);

  await call("POST", "/sign-in/magic-link", { body: { email: "linkonly@example.test", callbackURL: "/device?user_code=ABCD" } });
  const verify = await call("GET", outbox.at(-1).url);
  assert.equal(verify.status, 302);
  const location = new URL(verify.location, ORIGIN);
  assert.equal(location.pathname, "/account/two-factor");
  assert.equal(location.searchParams.get("next"), "/device?user_code=ABCD");
  const jar = cookieHeader(verify.setCookie);
  assert.doesNotMatch(jar, /session_token=/);
  assert.match(jar, /two_factor=/);
  assert.equal((await call("GET", "/get-session", { cookie: jar })).json, null);

  const done = await call("POST", "/two-factor/verify-totp", { cookie: jar, body: { code: await totpFor(totpURI) } });
  assert.equal(done.status, 200, JSON.stringify(done.json));
  assert.equal((await call("GET", "/get-session", { cookie: cookieHeader(jar, done.setCookie) })).json.user.email, "linkonly@example.test");
});

test("turning two-step off needs the password and sends a notice", async () => {
  const cookie = await verifiedAccount("off@example.test");
  const on = await turnOnTwoFactor(cookie, PASSWORD);
  outbox.length = 0;
  assert.equal((await call("POST", "/two-factor/disable", { cookie: on.cookie, body: { password: "wrong password!" } })).status, 400);
  const off = await call("POST", "/two-factor/disable", { cookie: on.cookie, body: { password: PASSWORD } });
  assert.equal(off.status, 200, JSON.stringify(off.json));
  assert.equal(db.user[0].twoFactorEnabled, false);
  assert.match(outbox.at(-1).subject, /turned off/);
});

test("passkeys need a signed in account to register", async () => {
  assert.equal((await call("GET", "/passkey/generate-register-options")).status, 401);
  const cookie = await verifiedAccount("pk@example.test");
  const options = await call("GET", "/passkey/generate-register-options", { cookie });
  assert.equal(options.status, 200, JSON.stringify(options.json));
  assert.equal(options.json.rp.id, "localhost");
  assert.equal(options.json.rp.name, "Commy");
  const list = await call("GET", "/passkey/list-user-passkeys", { cookie });
  assert.deepEqual(list.json, []);
});

test("return paths stay on this site", () => {
  assert.equal(safeReturnPath("/device?user_code=X", ORIGIN), "/device?user_code=X");
  assert.equal(safeReturnPath(`${ORIGIN}/account`, ORIGIN), "/account");
  assert.equal(safeReturnPath("https://evil.example/account", ORIGIN), "/account");
  assert.equal(safeReturnPath("//evil.example/x", ORIGIN), "/account");
  assert.equal(safeReturnPath(undefined, ORIGIN), "/account");
});

test("linking a social login to an existing account sends a notice", async () => {
  const sent = [];
  const notice = noticeWhenALoginIsLinked({ baseURL: ORIGIN, send: async (mail) => sent.push(mail) });
  const ctxWith = (accounts) => ({
    context: {
      internalAdapter: {
        findAccounts: async () => accounts,
        findUserById: async () => ({ email: "me@example.test" }),
      },
    },
  });
  await notice({ userId: "u1", providerId: "twitch" }, ctxWith([{ providerId: "twitch" }]));
  assert.equal(sent.length, 0, "a brand new account gets no notice");
  await notice({ userId: "u1", providerId: "credential" }, ctxWith([{}, {}]));
  assert.equal(sent.length, 0);
  await notice({ userId: "u1", providerId: "twitch" }, ctxWith([{ providerId: "credential" }, { providerId: "twitch" }]));
  assert.equal(sent.length, 1);
  assert.equal(sent[0].to, "me@example.test");
  assert.match(sent[0].subject, /Twitch sign-in was linked/);
});

function verifyTokenFor(email, expiresInSeconds) {
  const now = Math.floor(Date.now() / 1000);
  const part = (value) => Buffer.from(JSON.stringify(value)).toString("base64url");
  const unsigned = `${part({ alg: "HS256" })}.${part({ email, iat: now, exp: now + expiresInSeconds })}`;
  return `${unsigned}.${createHmac("sha256", SECRET).update(unsigned).digest("base64url")}`;
}

function verifyResultOf(location) {
  const page = new URL(location, ORIGIN);
  return { path: page.pathname, result: page.searchParams.get("result"), next: page.searchParams.get("next") };
}

test("the sign-up email link lands on a page that says the email is confirmed", async () => {
  await call("POST", "/sign-up/email", { body: { email: "fresh@example.test", password: PASSWORD, name: "Fresh", callbackURL: "/" } });
  const link = outbox.at(-1).url;
  const verify = await call("GET", link);
  assert.equal(verify.status, 302);
  assert.deepEqual(verifyResultOf(verify.location), { path: "/account/verify-email", result: "confirmed", next: "/" });
  assert.equal(db.user[0].emailVerified, true);
  const signedIn = await call("GET", "/get-session", { cookie: cookieHeader(verify.setCookie) });
  assert.equal(signedIn.json.user.email, "fresh@example.test");

  const again = await call("GET", link);
  assert.deepEqual(verifyResultOf(again.location), { path: "/account/verify-email", result: "confirmed", next: "/" });
});

test("an expired or broken verify link says so and keeps the return path", async () => {
  await call("POST", "/sign-up/email", { body: { email: "late@example.test", password: PASSWORD, name: "Late", callbackURL: "/web/" } });
  const callback = `callbackURL=${encodeURIComponent("/web/")}`;

  const expired = await call("GET", `/verify-email?token=${verifyTokenFor("late@example.test", -60)}&${callback}`);
  assert.deepEqual(verifyResultOf(expired.location), { path: "/account/verify-email", result: "expired", next: "/web/" });

  const broken = await call("GET", `/verify-email?token=${outbox.at(-1).url.split("token=")[1].split("&")[0].slice(0, -4)}&${callback}`);
  assert.deepEqual(verifyResultOf(broken.location), { path: "/account/verify-email", result: "invalid", next: "/web/" });

  const unknown = await call("GET", `/verify-email?token=${verifyTokenFor("nobody@example.test", 600)}&${callback}`);
  assert.deepEqual(verifyResultOf(unknown.location), { path: "/account/verify-email", result: "invalid", next: "/web/" });
  assert.equal(db.user[0].emailVerified, false);
});

test("verify links without a return path still answer with JSON", async () => {
  const r = await call("GET", `/verify-email?token=${verifyTokenFor("late@example.test", -60)}`);
  assert.equal(r.status, 401);
  assert.equal(r.location, null);
});

test("a verify link for a two-step account asks for the code before the result page", async () => {
  const cookie = await verifiedAccount("guarded@example.test");
  await turnOnTwoFactor(cookie, PASSWORD);
  db.user[0].emailVerified = false;
  const verify = await call("GET", `/verify-email?token=${verifyTokenFor("guarded@example.test", 600)}&callbackURL=%2F`);
  const page = new URL(verify.location, ORIGIN);
  assert.equal(page.pathname, "/account/two-factor");
  assert.deepEqual(verifyResultOf(page.searchParams.get("next")), { path: "/account/verify-email", result: "confirmed", next: "/" });
});
