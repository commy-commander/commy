import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { createSealer, sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { loadConfig } from "../src/config.js";
import { preferencesUrlMaker } from "../src/email-links.js";

const ORIGIN = "http://localhost:3001";
const PASSWORD = "correct horse battery";
const SECRET = "test-secret-".repeat(4);
const preferencesUrl = preferencesUrlMaker({ secret: SECRET, baseURL: ORIGIN, basePath: "/api/auth" });
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
    invite: [],
    inviteBackground: [],
    doorbellRing: [],
    twoFactor: [],
    passkey: [],
  };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  auth = betterAuth(options);
});

function cookieFrom(setCookie) {
  return (setCookie ?? "")
    .split(/,(?=\s*[\w.-]+=)/)
    .map((part) => part.trim().split(";")[0])
    .join("; ");
}

async function request(method, path, { json, form, cookie, origin = ORIGIN } = {}) {
  const url = path.startsWith("http") ? path : `${ORIGIN}/api/auth${path}`;
  const headers = {};
  if (origin) headers.origin = origin;
  if (cookie) headers.cookie = cookie;
  let body;
  if (json) {
    headers["content-type"] = "application/json";
    body = JSON.stringify(json);
  } else if (form) {
    headers["content-type"] = "application/x-www-form-urlencoded";
    body = new URLSearchParams(form).toString();
  }
  const res = await auth.handler(new Request(url, { method, headers, body }));
  const text = await res.text();
  return { status: res.status, text, headers: res.headers, setCookie: res.headers.get("set-cookie") };
}

async function verifiedAccount(email) {
  await request("POST", "/sign-up/email", { json: { email, password: PASSWORD, name: "Tester", callbackURL: "/account" } });
  await request("GET", outbox.at(-1).url);
  const signedIn = await request("POST", "/sign-in/email", { json: { email, password: PASSWORD } });
  assert.equal(signedIn.status, 200, signedIn.text);
  outbox.length = 0;
  return cookieFrom(signedIn.setCookie);
}

const userRow = (email) => db.user.find((u) => storedEmail(u) === email);

async function changePassword(cookie, from, to) {
  const r = await request("POST", "/change-password", { cookie, json: { currentPassword: from, newPassword: to } });
  assert.equal(r.status, 200, r.text);
}

const marketingOff = (email) => Boolean(userRow(email).emailMarketingOff);

test("the email link opens the settings page and changes nothing by itself", async () => {
  await verifiedAccount("reader@example.test");
  const page = await request("GET", preferencesUrl("Reader@Example.test"), { origin: null });
  assert.equal(page.status, 200);
  assert.match(page.headers.get("content-type"), /text\/html/);
  assert.match(page.headers.get("content-security-policy"), /default-src 'none'/);
  assert.equal(page.headers.get("referrer-policy"), "no-referrer");
  assert.match(page.text, /Manage your emails/);
  assert.match(page.text, /reader@example\.test/);
  assert.match(page.text, /Account emails/);
  assert.match(page.text, /Always on\./);
  assert.match(page.text, /Marketing emails/);
  assert.match(page.text, /name="choice" value="off"/);
  assert.equal((page.text.match(/<form/g) ?? []).length, 1, "only marketing has a switch");
  assert.equal(marketingOff("reader@example.test"), false);
});

test("the unsubscribe button turns marketing emails off", async () => {
  await verifiedAccount("button@example.test");
  const done = await request("POST", preferencesUrl("button@example.test"), { form: { choice: "off" } });
  assert.equal(done.status, 200);
  assert.match(done.text, /Saved\. You&#39;re unsubscribed from marketing emails\./);
  assert.match(done.text, /name="choice" value="on"/);
  assert.equal(marketingOff("button@example.test"), true);
});

test("mail clients can unsubscribe in one click without a session or origin", async () => {
  await verifiedAccount("oneclick@example.test");
  const r = await request("POST", preferencesUrl("oneclick@example.test"), {
    origin: null,
    form: { "List-Unsubscribe": "One-Click" },
  });
  assert.equal(r.status, 200);
  assert.equal(marketingOff("oneclick@example.test"), true);
});

test("a signed-in browser can use the page too", async () => {
  const cookie = await verifiedAccount("browser@example.test");
  const r = await request("POST", preferencesUrl("browser@example.test"), { cookie, form: { choice: "off" } });
  assert.equal(r.status, 200, r.text);
  assert.equal(marketingOff("browser@example.test"), true);
});

test("unsubscribing keeps every account email coming", async () => {
  const cookie = await verifiedAccount("quiet@example.test");
  await request("POST", preferencesUrl("quiet@example.test"), { form: { choice: "off" } });

  await changePassword(cookie, PASSWORD, "a brand new passphrase");
  assert.equal(outbox.length, 1);
  assert.match(outbox[0].subject, /password was changed/);

  const reset = await request("POST", "/request-password-reset", { json: { email: "quiet@example.test", redirectTo: "/account" } });
  assert.equal(reset.status, 200, reset.text);
  assert.equal(outbox.length, 2);
  assert.match(outbox[1].subject, /Reset your/);
});

test("subscribing again turns marketing back on", async () => {
  await verifiedAccount("back@example.test");
  const link = preferencesUrl("back@example.test");
  await request("POST", link, { form: { choice: "off" } });
  const on = await request("POST", link, { form: { choice: "on" } });
  assert.match(on.text, /Saved\. Marketing emails are back on\./);
  assert.equal(marketingOff("back@example.test"), false);
});

test("a changed or foreign token is refused and changes nothing", async () => {
  await verifiedAccount("victim@example.test");
  const link = new URL(preferencesUrl("victim@example.test"));
  link.searchParams.set("t", "A".repeat(link.searchParams.get("t").length));
  const r = await request("POST", link.toString(), { form: { choice: "off" } });
  assert.equal(r.status, 400);
  assert.match(r.text, /This link doesn&#39;t work/);
  assert.doesNotMatch(r.text, /<form/);

  const other = new URL(preferencesUrl("someone-else@example.test"));
  other.searchParams.set("a", new URL(preferencesUrl("victim@example.test")).searchParams.get("a"));
  assert.equal((await request("POST", other.toString(), { form: { choice: "off" } })).status, 400);

  assert.equal((await request("GET", `${ORIGIN}/api/auth/commy/email/preferences`)).status, 400);
  assert.equal(marketingOff("victim@example.test"), false);
});

test("an address without an account gets a calm answer", async () => {
  const r = await request("POST", preferencesUrl("nobody@example.test"), { form: { choice: "off" } });
  assert.equal(r.status, 200);
  assert.match(r.text, /unsubscribed from marketing/);
  assert.equal(db.user.length, 0);
});

test("the page escapes the address it shows", async () => {
  const r = await request("GET", preferencesUrl(`"><script>x</script>@example.test`));
  assert.equal(r.status, 200);
  assert.doesNotMatch(r.text, /<script>/);
});

test("the preference stays out of session answers", async () => {
  const cookie = await verifiedAccount("private@example.test");
  const session = await request("GET", "/get-session", { cookie });
  assert.equal(session.status, 200);
  assert.equal(session.text.includes("emailMarketingOff"), false);
});
