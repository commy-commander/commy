// Email verification and password reset against an in-memory database, with
// SMTP configured and outgoing mail captured instead of sent.
import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { buildAuthOptions } from "../src/auth.js";
import { loadConfig } from "../src/config.js";

const ORIGIN = "http://localhost:3001";
const PASSWORD = "correct horse battery";
let auth;
let outbox;

beforeEach(() => {
  const config = loadConfig({
    BETTER_AUTH_SECRET: "test-secret-".repeat(4),
    DATABASE_URL: "postgres://unused",
    BETTER_AUTH_URL: ORIGIN,
    SMTP_HOST: "smtp.invalid",
    SMTP_USER: "no-reply@example.test",
    SMTP_PASSWORD: "unused",
  });
  outbox = [];
  const options = buildAuthOptions(config, { pool: null, send: async (mail) => outbox.push(mail) });
  options.database = memoryAdapter({
    user: [],
    session: [],
    account: [],
    verification: [],
    deviceCode: [],
    linkedDevice: [],
    deviceRequest: [],
    invite: [],
    doorbellRing: [],
  });
  options.rateLimit = { enabled: false };
  auth = betterAuth(options);
});

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

const signIn = (email, password = PASSWORD) => call("POST", "/sign-in/email", { body: { email, password } });

test("sign-up sends a verification email and blocks sign-in until verified", async () => {
  const up = await call("POST", "/sign-up/email", {
    body: { email: "new@example.test", password: PASSWORD, name: "New", callbackURL: "/account" },
  });
  assert.equal(up.status, 200, JSON.stringify(up.json));
  assert.equal(up.json.token, null);
  assert.equal(outbox.length, 1);
  assert.equal(outbox[0].to, "new@example.test");
  assert.match(outbox[0].subject, /Verify/);

  const blocked = await signIn("new@example.test");
  assert.equal(blocked.status, 403);
  assert.equal(blocked.json.code, "EMAIL_NOT_VERIFIED");
  // The failed attempt sends a fresh link
  assert.equal(outbox.length, 2);

  const resend = await call("POST", "/send-verification-email", {
    body: { email: "new@example.test", callbackURL: "/account" },
  });
  assert.equal(resend.status, 200, JSON.stringify(resend.json));
  assert.equal(outbox.length, 3);

  const verify = await call("GET", outbox[2].url);
  assert.equal(verify.status, 302);
  assert.match(verify.setCookie ?? "", /session_token=/);

  const ok = await signIn("new@example.test");
  assert.equal(ok.status, 200, JSON.stringify(ok.json));
  assert.equal(ok.json.user.emailVerified, true);
});

test("resending for an unknown address reveals nothing and sends nothing", async () => {
  const r = await call("POST", "/send-verification-email", { body: { email: "nobody@example.test" } });
  assert.equal(r.status, 200);
  assert.equal(outbox.length, 0);
});

test("password reset sets a new password and ends old sessions", async () => {
  await call("POST", "/sign-up/email", { body: { email: "r@example.test", password: PASSWORD, name: "R" } });
  await call("GET", outbox[0].url);
  const session = await signIn("r@example.test");
  assert.equal(session.status, 200, JSON.stringify(session.json));
  const cookie = session.setCookie.split(";")[0];

  const req = await call("POST", "/request-password-reset", {
    body: { email: "r@example.test", redirectTo: `${ORIGIN}/account/reset-password` },
  });
  assert.equal(req.status, 200, JSON.stringify(req.json));
  const mail = outbox.at(-1);
  assert.match(mail.subject, /Reset/);

  const link = await call("GET", mail.url);
  assert.equal(link.status, 302);
  const token = new URL(link.location, ORIGIN).searchParams.get("token");
  assert.ok(token);

  const reset = await call("POST", "/reset-password", { body: { token, newPassword: "a whole new passphrase" } });
  assert.equal(reset.status, 200, JSON.stringify(reset.json));

  assert.equal((await signIn("r@example.test")).status, 401);
  assert.equal((await signIn("r@example.test", "a whole new passphrase")).status, 200);
  const old = await call("GET", "/get-session", { cookie });
  assert.equal(old.json, null);

  const reuse = await call("POST", "/reset-password", { body: { token, newPassword: "yet another passphrase" } });
  assert.equal(reuse.status, 400);
});
