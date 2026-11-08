import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";
import { createHmac, generateKeyPairSync, verify } from "node:crypto";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { accessToken } from "../src/access.js";
import { captchaPage } from "../src/app-sign-in.js";
import { appleClientSecret, signAppleClientSecret } from "../src/apple.js";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";

const ORIGIN = "http://localhost:3001";
const PASSWORD = "correct horse battery";
const SECRET = "test-secret-".repeat(4);
const NONCE = "nonce-from-the-app";
const BASE_ENV = { BETTER_AUTH_SECRET: SECRET, DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN };
let auth;
let db;

function start(env = {}) {
  const config = loadConfig({ ...BASE_ENV, ...env });
  const options = buildAuthOptions(config, { pool: null, send: async () => {} });
  db = { user: [], session: [], account: [], verification: [], twoFactor: [], passkey: [], deviceCode: [], linkedDevice: [] };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  options.socialProviders.apple.verifyIdToken = async (_token, nonce) => nonce === NONCE;
  auth = betterAuth(options);
}

beforeEach(() => start());

function appleToken(claims) {
  const part = (value) => Buffer.from(JSON.stringify(value)).toString("base64url");
  return `${part({ alg: "RS256", kid: "test" })}.${part({ iss: "https://appleid.apple.com", aud: "chat.commy.mobile", email_verified: true, ...claims })}.signature`;
}

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

async function call(method, path, { body, cookie, origin = ORIGIN } = {}) {
  const headers = {};
  if (origin) headers.origin = origin;
  if (body) headers["content-type"] = "application/json";
  if (cookie) headers.cookie = cookie;
  const res = await auth.handler(new Request(`${ORIGIN}/api/auth${path}`, { method, headers, body: body ? JSON.stringify(body) : undefined }));
  const text = await res.text();
  let json = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    json = null;
  }
  return { status: res.status, json, text, headers: res.headers, setCookie: res.headers.get("set-cookie") };
}

function signInWithApple(claims, { requestSignUp = false, cookie, nonce = NONCE } = {}) {
  return call("POST", "/sign-in/social", {
    origin: cookie ? ORIGIN : null,
    cookie,
    body: { provider: "apple", idToken: { token: appleToken(claims), nonce }, ...(requestSignUp ? { requestSignUp: true } : {}) },
  });
}

async function passwordAccount(email, { verified = true } = {}) {
  const made = await call("POST", "/sign-up/email", { body: { email, password: PASSWORD, name: "Tester" } });
  assert.equal(made.status, 200, JSON.stringify(made.json));
  db.user.at(-1).emailVerified = verified;
  return cookieHeader(made.setCookie);
}

const BASE32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

function totpFor(totpURI) {
  let bits = "";
  for (const c of new URL(totpURI).searchParams.get("secret").replace(/=+$/, "")) bits += BASE32.indexOf(c).toString(2).padStart(5, "0");
  const bytes = [];
  for (let i = 0; i + 8 <= bits.length; i += 8) bytes.push(parseInt(bits.slice(i, i + 8), 2));
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(Math.floor(Date.now() / 30000)));
  const mac = createHmac("sha1", Buffer.from(bytes)).update(counter).digest();
  const offset = mac[mac.length - 1] & 15;
  return String((mac.readUInt32BE(offset) & 0x7fffffff) % 1_000_000).padStart(6, "0");
}

test("an Apple ID with no account is not signed up until the app asks for it", async () => {
  const claims = { sub: "apple-new", email: "x7k2@privaterelay.appleid.com", is_private_email: true };
  const first = await signInWithApple(claims);
  assert.equal(first.status, 401, JSON.stringify(first.json));
  assert.match(first.json.message, /signup disabled/);
  assert.equal(db.user.length, 0);

  const created = await signInWithApple(claims, { requestSignUp: true });
  assert.equal(created.status, 200, JSON.stringify(created.json));
  assert.ok(created.json.token);
  assert.ok(created.headers.get("set-auth-token"), "the app gets a bearer token");
  assert.equal(db.user.length, 1);

  const again = await signInWithApple(claims);
  assert.equal(again.status, 200, JSON.stringify(again.json));
  assert.equal(db.user.length, 1);
});

test("an Apple ID with the same verified email joins the existing account", async () => {
  await passwordAccount("same@example.test");
  const r = await signInWithApple({ sub: "apple-same", email: "same@example.test" });
  assert.equal(r.status, 200, JSON.stringify(r.json));
  assert.equal(db.user.length, 1);
  assert.deepEqual(db.account.map((a) => a.providerId).sort(), ["apple", "credential"]);
});

test("an Apple ID never joins an account whose email was not verified", async () => {
  await passwordAccount("unverified@example.test", { verified: false });
  const r = await signInWithApple({ sub: "apple-squat", email: "unverified@example.test" }, { requestSignUp: true });
  assert.equal(r.status, 401);
  assert.equal(r.json.message, "account not linked");
  assert.deepEqual(db.account.map((a) => a.providerId), ["credential"]);
});

test("a token with the wrong nonce is refused", async () => {
  const r = await signInWithApple({ sub: "apple-x", email: "x@example.test" }, { requestSignUp: true, nonce: "replayed" });
  assert.equal(r.status, 401);
  assert.equal(db.user.length, 0);
});

test("Sign in with Apple still asks for the authenticator code", async () => {
  const cookie = await passwordAccount("twostep@example.test");
  const enabled = await call("POST", "/two-factor/enable", { cookie, body: { password: PASSWORD } });
  assert.equal(enabled.status, 200, JSON.stringify(enabled.json));
  await call("POST", "/two-factor/verify-totp", { cookie, body: { code: totpFor(enabled.json.totpURI) } });

  const first = await signInWithApple({ sub: "apple-2fa", email: "twostep@example.test" });
  assert.equal(first.status, 200, JSON.stringify(first.json));
  assert.equal(first.json.twoFactorRedirect, true);
  assert.equal(first.json.token, undefined);
  assert.doesNotMatch(cookieHeader(first.setCookie), /session_token=/);

  const right = await call("POST", "/two-factor/verify-totp", { cookie: cookieHeader(first.setCookie), body: { code: totpFor(enabled.json.totpURI) } });
  assert.equal(right.status, 200, JSON.stringify(right.json));
  assert.ok(right.json.token);
});

test("the app signs in with email and password without a browser", async () => {
  await passwordAccount("app@example.test");
  const r = await call("POST", "/sign-in/email", { origin: null, body: { email: "app@example.test", password: PASSWORD } });
  assert.equal(r.status, 200, JSON.stringify(r.json));
  const token = r.headers.get("set-auth-token");
  assert.ok(token);
  const session = await auth.handler(new Request(`${ORIGIN}/api/auth/get-session`, { headers: { authorization: `Bearer ${token}` } }));
  assert.equal((await session.json()).user.email, "app@example.test");
});

test("the access code check answers with the cookie value and Apple sign-up needs it", async () => {
  start({ ACCESS_CODE: "Open Sesame" });
  const wrong = await call("POST", "/commy/access", { origin: null, body: { code: "nope" } });
  assert.equal(wrong.status, 403);
  assert.equal(wrong.json.code, "WRONG_ACCESS_CODE");
  const right = await call("POST", "/commy/access", { origin: null, body: { code: "  open sesame " } });
  assert.equal(right.status, 200);
  assert.equal(right.json.token, accessToken("open sesame"));

  const claims = { sub: "apple-gated", email: "gated@example.test" };
  const refused = await signInWithApple(claims, { requestSignUp: true });
  assert.notEqual(refused.status, 200);
  assert.equal(db.user.length, 0);
  const allowed = await signInWithApple(claims, { requestSignUp: true, cookie: `commy_access=${right.json.token}` });
  assert.equal(allowed.status, 200, JSON.stringify(allowed.json));
  assert.equal(db.user.length, 1);
});

test("the in-app check page exists only with Turnstile and only frames inside the app", async () => {
  assert.equal((await call("GET", "/commy/captcha")).status, 404);
  start({ TURNSTILE_SITE_KEY: "0xSITE", TURNSTILE_SECRET_KEY: "0xSECRET" });
  const page = await call("GET", "/commy/captcha");
  assert.equal(page.status, 200);
  assert.match(page.headers.get("content-type"), /text\/html/);
  assert.equal(page.headers.get("content-security-policy"), "frame-ancestors tauri://localhost http://tauri.localhost https://tauri.localhost");
  assert.match(page.text, /"0xSITE"/);
  assert.doesNotMatch(captchaPage("</script><b>"), /<\/script><b>/);
});

test("Apple sign-in for the Commy app is on by default and can be turned off", () => {
  const on = loadConfig(BASE_ENV);
  assert.deepEqual(on.apple, { appBundleIds: ["chat.commy.mobile"], web: null });
  const doc = discoveryDocument(on);
  assert.equal(doc.providers.appleApp, true);
  assert.equal(doc.providers.apple, false);
  assert.equal(doc.endpoints.signInEmail, `${ORIGIN}/api/auth/sign-in/email`);
  assert.equal(doc.endpoints.signInSocial, `${ORIGIN}/api/auth/sign-in/social`);
  assert.equal(doc.endpoints.captcha, undefined);
  assert.equal(doc.endpoints.accessCode, undefined);
  assert.ok(!on.trustedOrigins.includes("https://appleid.apple.com"));

  const off = loadConfig({ ...BASE_ENV, APPLE_APP_BUNDLE_IDS: "off" });
  assert.equal(off.apple, null);
  assert.equal(discoveryDocument(off).providers.appleApp, false);
});

test("website Apple sign-in needs all four values and trusts Apple's form post", () => {
  const half = loadConfig({ ...BASE_ENV, APPLE_CLIENT_ID: "chat.commy.web" });
  assert.equal(half.apple.web, null);
  assert.match(half.warnings.join("\n"), /APPLE_TEAM_ID, APPLE_KEY_ID, APPLE_PRIVATE_KEY empty/);

  const { privateKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
  const pem = privateKey.export({ type: "pkcs8", format: "pem" }).replace(/\n/g, "\\n");
  const full = loadConfig({ ...BASE_ENV, APPLE_CLIENT_ID: "chat.commy.web", APPLE_TEAM_ID: "TEAM123456", APPLE_KEY_ID: "KEY1234567", APPLE_PRIVATE_KEY: pem });
  assert.equal(full.apple.web.servicesId, "chat.commy.web");
  assert.ok(full.trustedOrigins.includes("https://appleid.apple.com"));
  assert.equal(discoveryDocument(full).providers.apple, true);

  assert.throws(() => loadConfig({ ...BASE_ENV, APPLE_CLIENT_ID: "a", APPLE_TEAM_ID: "b", APPLE_KEY_ID: "c", APPLE_PRIVATE_KEY: "not a key" }), /APPLE_PRIVATE_KEY/);
});

test("the Apple client secret is a signed ES256 token that renews itself", () => {
  const { privateKey, publicKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
  const web = { servicesId: "chat.commy.web", teamId: "TEAM123456", keyId: "KEY1234567", privateKey: privateKey.export({ type: "pkcs8", format: "pem" }) };
  const jwt = signAppleClientSecret(web, 1_000);
  const [header, claims, signature] = jwt.split(".");
  assert.deepEqual(JSON.parse(Buffer.from(header, "base64url")), { alg: "ES256", kid: "KEY1234567", typ: "JWT" });
  const body = JSON.parse(Buffer.from(claims, "base64url"));
  assert.deepEqual(body, { iss: "TEAM123456", iat: 1_000, exp: 1_000 + 150 * 24 * 3600, aud: "https://appleid.apple.com", sub: "chat.commy.web" });
  assert.ok(verify("sha256", Buffer.from(`${header}.${claims}`), { key: publicKey, dsaEncoding: "ieee-p1363" }, Buffer.from(signature, "base64url")));

  let now = 1_000;
  const secret = appleClientSecret(web, () => now);
  const first = secret.current();
  now += 29 * 24 * 3600;
  assert.equal(secret.current(), first);
  now += 2 * 24 * 3600;
  assert.notEqual(secret.current(), first);
});
