import assert from "node:assert/strict";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { ACCESS_COOKIE, accessToken } from "../src/access.js";
import { confirmedAdult } from "../src/adult-confirmation.js";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, featureSummary, loadConfig } from "../src/config.js";

const ORIGIN = "http://localhost:3001";

function makeAuth(env = {}, outbox = []) {
  const config = loadConfig({
    BETTER_AUTH_SECRET: "test-secret-".repeat(4),
    DATABASE_URL: "postgres://unused",
    BETTER_AUTH_URL: ORIGIN,
    ...env,
  });
  const options = buildAuthOptions(config, { pool: null, send: async (mail) => outbox.push(mail) });
  const db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], inviteBackground: [], doorbellRing: [] };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  return { auth: betterAuth(options), db, config };
}

async function signUp(auth, extra = {}, cookie) {
  const headers = { origin: ORIGIN, "content-type": "application/json" };
  if (cookie) headers.cookie = cookie;
  const body = { email: `${Math.random().toString(36).slice(2)}@example.com`, password: "correct horse battery", name: "Tester", ...extra };
  const res = await auth.handler(new Request(`${ORIGIN}/api/auth/sign-up/email`, { method: "POST", headers, body: JSON.stringify(body) }));
  const text = await res.text();
  return { status: res.status, json: text ? JSON.parse(text) : null };
}

function cookiesFrom(res) {
  return res.headers.getSetCookie().map((c) => c.split(";")[0]).join("; ");
}

function unsignedJwt(claims) {
  const part = (o) => Buffer.from(JSON.stringify(o)).toString("base64url");
  return `${part({ alg: "none", typ: "JWT" })}.${part(claims)}.`;
}

function fakeGoogle(email) {
  const realFetch = globalThis.fetch;
  globalThis.fetch = async (input, init) => {
    const url = String(input instanceof Request ? input.url : input);
    if (!url.startsWith("https://oauth2.googleapis.com/token")) return realFetch(input, init);
    const idToken = unsignedJwt({ sub: `g-${email}`, email, email_verified: true, name: "Googler", iss: "https://accounts.google.com", aud: "gid", exp: Math.floor(Date.now() / 1000) + 600 });
    return new Response(JSON.stringify({ access_token: "at", id_token: idToken, token_type: "Bearer", expires_in: 600 }), { headers: { "content-type": "application/json" } });
  };
  return () => {
    globalThis.fetch = realFetch;
  };
}

async function googleSignUp(auth, additionalData) {
  const start = await auth.handler(
    new Request(`${ORIGIN}/api/auth/sign-in/social`, {
      method: "POST",
      headers: { origin: ORIGIN, "content-type": "application/json" },
      body: JSON.stringify({ provider: "google", callbackURL: "/account", errorCallbackURL: "/account", ...(additionalData ? { additionalData } : {}) }),
    }),
  );
  const { url } = await start.json();
  const state = new URL(url).searchParams.get("state");
  const done = await auth.handler(
    new Request(`${ORIGIN}/api/auth/callback/google?code=c&state=${encodeURIComponent(state)}`, { headers: { cookie: cookiesFrom(start) } }),
  );
  return done.headers.get("location") ?? "";
}

async function magicLinkSignUp(auth, outbox, email, metadata) {
  await auth.handler(
    new Request(`${ORIGIN}/api/auth/sign-in/magic-link`, {
      method: "POST",
      headers: { origin: ORIGIN, "content-type": "application/json" },
      body: JSON.stringify({ email, callbackURL: "/account", errorCallbackURL: "/account", ...(metadata ? { metadata } : {}) }),
    }),
  );
  const res = await auth.handler(new Request(outbox.at(-1).url));
  return res.headers.get("location") ?? "";
}

const ctxWith = (body) => ({ body });

test("confirmedAdult accepts only a literal true from the app or a social sign-up", async () => {
  assert.equal(await confirmedAdult({}, ctxWith({ adultConfirmed: true })), true);
  assert.equal(await confirmedAdult({}, ctxWith({ additionalData: { adultConfirmed: true } })), true);
  assert.equal(await confirmedAdult({}, ctxWith({ adultConfirmed: "true" })), false);
  assert.equal(await confirmedAdult({}, ctxWith({})), false);
  assert.equal(await confirmedAdult(undefined, undefined), false);
});

test("with ADULTS_ONLY, sign-up without the confirmation is refused and stores no user", async () => {
  const { auth, db } = makeAuth({ ADULTS_ONLY: "true" });
  const res = await signUp(auth);
  assert.equal(res.status, 403, JSON.stringify(res.json));
  assert.equal(res.json.code, "ADULT_CONFIRMATION_REQUIRED");
  assert.equal(db.user.length, 0);
});

test("with ADULTS_ONLY, a confirmed email sign-up creates the account and stores when", async () => {
  const { auth, db } = makeAuth({ ADULTS_ONLY: "true", TERMS_VERSION: " 2026-10-25 " });
  const before = Date.now();
  const res = await signUp(auth, { adultConfirmed: true });
  assert.equal(res.status, 200, JSON.stringify(res.json));
  assert.equal(db.user.length, 1);
  assert.ok(db.user[0].adultConfirmedAt instanceof Date);
  assert.ok(db.user[0].adultConfirmedAt.getTime() >= before);
  assert.equal(db.user[0].termsAcceptedAt.getTime(), db.user[0].adultConfirmedAt.getTime());
  assert.equal(db.user[0].termsVersion, "2026-10-25");
});

test("with ADULTS_ONLY, a Google sign-up needs the confirmation carried through the provider redirect", async () => {
  const restore = fakeGoogle("g@example.com");
  try {
    const { auth, db } = makeAuth({ ADULTS_ONLY: "true", GOOGLE_CLIENT_ID: "gid", GOOGLE_CLIENT_SECRET: "gsecret" });
    assert.match(await googleSignUp(auth), /error=adult_confirmation_required/i);
    assert.equal(db.user.length, 0);
    assert.doesNotMatch(await googleSignUp(auth, { adultConfirmed: true }), /error=/);
    assert.equal(db.user.length, 1);
    assert.ok(db.user[0].adultConfirmedAt instanceof Date);
  } finally {
    restore();
  }
});

test("with ADULTS_ONLY, a magic-link sign-up remembers the confirmation until the link is opened", async () => {
  const outbox = [];
  const { auth, db } = makeAuth({ ADULTS_ONLY: "true", SMTP_HOST: "smtp.invalid", SMTP_USER: "no-reply@example.test", SMTP_PASSWORD: "unused" }, outbox);
  assert.match(await magicLinkSignUp(auth, outbox, "m1@example.com"), /error=/i);
  assert.equal(db.user.length, 0);
  assert.doesNotMatch(await magicLinkSignUp(auth, outbox, "m2@example.com", { adultConfirmed: true }), /error=/);
  assert.equal(db.user.length, 1);
  assert.ok(db.user[0].adultConfirmedAt instanceof Date);
});

test("the access code is still checked before the age confirmation", async () => {
  const { auth, db } = makeAuth({ ADULTS_ONLY: "true", ACCESS_CODE: "gabbro" });
  const noCode = await signUp(auth, { adultConfirmed: true });
  assert.equal(noCode.json.code, "ACCESS_CODE_REQUIRED");
  const both = await signUp(auth, { adultConfirmed: true }, `${ACCESS_COOKIE}=${accessToken("gabbro")}`);
  assert.equal(both.status, 200, JSON.stringify(both.json));
  assert.equal(db.user.length, 1);
});

test("a client cannot write the confirmation time itself", async () => {
  const { auth, db } = makeAuth({ ADULTS_ONLY: "true" });
  const forgedFields = [
    { adultConfirmedAt: "2000-01-01T00:00:00Z" },
    { termsAcceptedAt: "2000-01-01T00:00:00Z" },
    { termsVersion: "old" },
  ];
  for (const forged of forgedFields) {
    const res = await signUp(auth, { adultConfirmed: true, ...forged });
    assert.equal(res.status, 400, JSON.stringify(res.json));
  }
  assert.equal(db.user.length, 0);
});

test("without ADULTS_ONLY, sign-up works as before and still records a confirmation it was given", async () => {
  const { auth, db } = makeAuth();
  assert.equal((await signUp(auth)).status, 200);
  assert.equal((await signUp(auth, { adultConfirmed: true })).status, 200);
  assert.equal(db.user[0].adultConfirmedAt ?? null, null);
  assert.ok(db.user[1].adultConfirmedAt instanceof Date);
});

test("ADULTS_ONLY shows in the startup summary and the discovery document", () => {
  const on = makeAuth({ ADULTS_ONLY: "true" }).config;
  assert.match(featureSummary(on), /adults_only=on/);
  assert.equal(discoveryDocument(on).features.adultsOnly, true);
  const off = makeAuth().config;
  assert.match(featureSummary(off), /adults_only=off/);
  assert.equal(discoveryDocument(off).features.adultsOnly, false);
});
