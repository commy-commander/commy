import assert from "node:assert/strict";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { featureSummary, loadConfig } from "../src/config.js";
import { emailDomain, isThrowawayEmail, loadThrowawayDomains } from "../src/throwaway-email.js";

const ORIGIN = "http://localhost:3001";
const MAIL = { SMTP_HOST: "smtp.invalid", SMTP_USER: "no-reply@example.test", SMTP_PASSWORD: "unused" };
const PASSWORD = "correct horse battery";

function makeAuth(env = {}) {
  const config = loadConfig({ BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN, ...env });
  const outbox = [];
  const options = buildAuthOptions(config, { pool: null, send: async (mail) => outbox.push(mail) });
  const db = { user: [], session: [], commyIdentity: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], inviteBackground: [], doorbellRing: [] };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  return { auth: betterAuth(options), db, outbox };
}

async function post(auth, path, body, cookie) {
  const headers = { origin: ORIGIN, "content-type": "application/json", ...(cookie ? { cookie } : {}) };
  const res = await auth.handler(new Request(`${ORIGIN}/api/auth${path}`, { method: "POST", headers, body: JSON.stringify(body) }));
  const text = await res.text();
  return { res, json: text ? JSON.parse(text) : null };
}

const cookiesFrom = (res) => res.headers.getSetCookie().map((c) => c.split(";")[0]).join("; ");

test("the bundled list knows common throwaway services and leaves everyday mail alone", () => {
  const domains = loadThrowawayDomains();
  assert.ok(domains.size > 1000);
  assert.equal(isThrowawayEmail("someone@mailinator.com", domains), true);
  assert.equal(isThrowawayEmail("Someone@Inbox.MAILINATOR.com.", domains), true);
  for (const everyday of ["gmail.com", "outlook.com", "icloud.com", "proton.me", "privaterelay.appleid.com", "duck.com", "fastmail.com"]) {
    assert.equal(isThrowawayEmail(`someone@${everyday}`, domains), false, everyday);
  }
  assert.equal(isThrowawayEmail("not-an-email", domains), false);
  assert.equal(emailDomain(null), "");
});

test("email sign-up refuses a throwaway address and stores nothing", async () => {
  const { auth, db, outbox } = makeAuth(MAIL);
  const refused = await post(auth, "/sign-up/email", { email: "spam@mailinator.com", password: PASSWORD, name: "Spam" });
  assert.equal(refused.res.status, 400);
  assert.equal(refused.json.code, "THROWAWAY_EMAIL");
  assert.equal(db.user.length, 0);
  assert.equal(outbox.length, 0);

  const allowed = await post(auth, "/sign-up/email", { email: "real@example.com", password: PASSWORD, name: "Real" });
  assert.equal(allowed.res.status, 200);
  assert.equal(db.user.length, 1);
});

test("a magic link for a new throwaway address is refused before any email goes out", async () => {
  const { auth, outbox } = makeAuth(MAIL);
  const refused = await post(auth, "/sign-in/magic-link", { email: "spam@mailinator.com", callbackURL: "/" });
  assert.equal(refused.res.status, 400);
  assert.equal(refused.json.code, "THROWAWAY_EMAIL");
  assert.equal(outbox.length, 0);

  const allowed = await post(auth, "/sign-in/magic-link", { email: "real@example.com", callbackURL: "/" });
  assert.equal(allowed.res.status, 200);
  assert.equal(outbox.length, 1);
});

test("moving an account to a throwaway address is refused", async () => {
  const { auth, db } = makeAuth(MAIL);
  await post(auth, "/sign-up/email", { email: "mover@example.com", password: PASSWORD, name: "Mover" });
  db.user[0].emailVerified = true;
  const signIn = await post(auth, "/sign-in/email", { email: "mover@example.com", password: PASSWORD });
  assert.equal(signIn.res.status, 200);
  const moved = await post(auth, "/change-email", { newEmail: "gone@mailinator.com", callbackURL: "/account" }, cookiesFrom(signIn.res));
  assert.equal(moved.res.status, 400);
  assert.equal(moved.json.code, "THROWAWAY_EMAIL");
});

test("BLOCK_THROWAWAY_EMAILS=false lets throwaway addresses sign up", async () => {
  const { auth, db } = makeAuth({ ...MAIL, BLOCK_THROWAWAY_EMAILS: "false" });
  const res = await post(auth, "/sign-up/email", { email: "fine@mailinator.com", password: PASSWORD, name: "Fine" });
  assert.equal(res.res.status, 200);
  assert.equal(db.user.length, 1);
});

test("the startup line says whether throwaway addresses are blocked and never-verified accounts are removed", () => {
  const base = { BETTER_AUTH_SECRET: "x".repeat(32), DATABASE_URL: "postgres://u:p@db:5432/auth", BETTER_AUTH_URL: ORIGIN };
  assert.match(featureSummary(loadConfig({ ...base, ...MAIL })), /throwaway_emails=blocked unverified_cleanup=on/);
  assert.match(featureSummary(loadConfig({ ...base, BLOCK_THROWAWAY_EMAILS: "false" })), /throwaway_emails=allowed unverified_cleanup=off/);
});
