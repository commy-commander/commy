import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { featureSummary, loadConfig } from "../src/config.js";
import { isKnownLeak, passwordHashParts, pwnedPasswordsLookup, rangeListsSuffix } from "../src/leaked-passwords.js";

const ORIGIN = "http://localhost:3001";
const SECRET = "test-secret-".repeat(4);
const LEAKED = "leaked passphrase 1";
const FRESH = "a fresh passphrase";
const BASE = { BETTER_AUTH_SECRET: SECRET, DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN };
let auth;
let db;
let outbox;
let lookups;
let lookupFails;

function quietLog() {
  const warnings = [];
  return { warnings, warn: (line) => warnings.push(line) };
}

beforeEach(() => {
  const config = loadConfig({ ...BASE, SMTP_HOST: "smtp.invalid", SMTP_USER: "no-reply@example.test", SMTP_PASSWORD: "unused" });
  outbox = [];
  lookups = [];
  lookupFails = false;
  const isLeakedPassword = async (password) => {
    lookups.push(password);
    if (lookupFails) throw new Error("offline");
    return password === LEAKED;
  };
  const options = buildAuthOptions(config, { pool: null, send: async (mail) => outbox.push(mail), isLeakedPassword });
  db = { user: [], session: [], commyIdentity: [], account: [], verification: [], twoFactor: [], passkey: [] };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  options.logger = { disabled: true };
  auth = betterAuth(options);
});

async function call(path, body, cookie) {
  const headers = { origin: ORIGIN, "content-type": "application/json" };
  if (cookie) headers.cookie = cookie;
  const res = await auth.handler(new Request(`${ORIGIN}/api/auth${path}`, { method: "POST", headers, body: JSON.stringify(body) }));
  const text = await res.text();
  return { status: res.status, json: text ? JSON.parse(text) : null, setCookie: res.headers.get("set-cookie") };
}

async function signedInAccount(email, password) {
  const signUp = await call("/sign-up/email", { email, password, name: "Tester" });
  assert.equal(signUp.status, 200, JSON.stringify(signUp.json));
  db.user[0].emailVerified = true;
  const signIn = await call("/sign-in/email", { email, password });
  assert.equal(signIn.status, 200, JSON.stringify(signIn.json));
  return signIn.setCookie.split(";")[0];
}

test("only the first five characters of the SHA-1 hash leave the server", async () => {
  assert.deepEqual(passwordHashParts("password"), { prefix: "5BAA6", suffix: "1E4C9B93F3F0682250B6CF8331B7EE68FD8" });
  const asked = [];
  const lookup = pwnedPasswordsLookup({
    fetch: async (url, init) => {
      asked.push({ url, padding: init.headers["Add-Padding"] });
      return new Response("1E4C9B93F3F0682250B6CF8331B7EE68FD8:52256179\r\n0018A45C4D1DEF81644B54AB7F969B88D65:0\r\n");
    },
  });
  assert.equal(await lookup("password"), true);
  assert.deepEqual(asked, [{ url: "https://api.pwnedpasswords.com/range/5BAA6", padding: "true" }]);
});

test("padding rows with a zero count do not count as leaks", () => {
  assert.equal(rangeListsSuffix("ABC:0\nDEF:3", "ABC"), false);
  assert.equal(rangeListsSuffix("abc:2", "ABC"), true);
  assert.equal(rangeListsSuffix("", "ABC"), false);
});

test("an unreachable leak check lets the password through and logs it", async () => {
  const log = quietLog();
  const failing = pwnedPasswordsLookup({ fetch: async () => new Response("busy", { status: 503 }) });
  assert.equal(await isKnownLeak(failing, "anything at all", log), false);
  assert.match(log.warnings[0], /leaked password check skipped: status 503/);
});

test("sign-up refuses a leaked password and a password under 12 characters", async () => {
  const leaked = await call("/sign-up/email", { email: "leak@example.test", password: LEAKED, name: "Tester" });
  assert.equal(leaked.status, 400);
  assert.equal(leaked.json.code, "PASSWORD_COMPROMISED");
  assert.match(leaked.json.message, /data breach/);
  const short = await call("/sign-up/email", { email: "short@example.test", password: "elevenchars", name: "Tester" });
  assert.equal(short.status, 400);
  assert.equal(short.json.code, "PASSWORD_TOO_SHORT");
  assert.equal(db.user.length, 0);
  const ok = await call("/sign-up/email", { email: "ok@example.test", password: FRESH, name: "Tester" });
  assert.equal(ok.status, 200, JSON.stringify(ok.json));
});

test("sign-up still works when the leak check is unreachable", async () => {
  lookupFails = true;
  const r = await call("/sign-up/email", { email: "offline@example.test", password: LEAKED, name: "Tester" });
  assert.equal(r.status, 200, JSON.stringify(r.json));
});

test("changing the password refuses a leaked one", async () => {
  const cookie = await signedInAccount("change@example.test", FRESH);
  const leaked = await call("/change-password", { currentPassword: FRESH, newPassword: LEAKED }, cookie);
  assert.equal(leaked.status, 400);
  assert.equal(leaked.json.code, "PASSWORD_COMPROMISED");
  const ok = await call("/change-password", { currentPassword: FRESH, newPassword: "another fresh passphrase" }, cookie);
  assert.equal(ok.status, 200, JSON.stringify(ok.json));
});

test("a reset refuses a leaked password and keeps the link usable", async () => {
  await signedInAccount("reset@example.test", FRESH);
  await call("/request-password-reset", { email: "reset@example.test", redirectTo: `${ORIGIN}/account/reset-password` });
  const token = new URL(outbox.at(-1).url).pathname.split("/").pop();
  const leaked = await call("/reset-password", { token, newPassword: LEAKED });
  assert.equal(leaked.status, 400);
  assert.equal(leaked.json.code, "PASSWORD_COMPROMISED");
  const ok = await call("/reset-password", { token, newPassword: "a whole new passphrase" });
  assert.equal(ok.status, 200, JSON.stringify(ok.json));
});

test("signing in never runs the leak check, so older shorter passwords keep working", async () => {
  await signedInAccount("old@example.test", FRESH);
  const account = db.account.find((a) => a.providerId === "credential");
  account.password = await (await auth.$context).password.hash("eight ch");
  lookups.length = 0;
  const r = await call("/sign-in/email", { email: "old@example.test", password: "eight ch" });
  assert.equal(r.status, 200, JSON.stringify(r.json));
  assert.deepEqual(lookups, []);
});

test("the leak check is on unless turned off", () => {
  assert.equal(loadConfig(BASE).checkLeakedPasswords, true);
  assert.equal(loadConfig({ ...BASE, CHECK_LEAKED_PASSWORDS: "false" }).checkLeakedPasswords, false);
  assert.match(featureSummary(loadConfig(BASE)), /leaked_passwords=on/);
  assert.match(featureSummary(loadConfig({ ...BASE, CHECK_LEAKED_PASSWORDS: "false" })), /leaked_passwords=off/);
});

test("the account pages ask for the same password length as the server", async () => {
  const server = await import("../src/password-rules.js");
  const pages = await import("../pages/src/lib/password-rules.js");
  assert.equal(pages.MIN_NEW_PASSWORD, server.MIN_NEW_PASSWORD);
  assert.equal(pages.MAX_PASSWORD, server.MAX_PASSWORD);
  assert.match(pages.NEW_PASSWORD_HINT, /At least 12 characters/);
});
