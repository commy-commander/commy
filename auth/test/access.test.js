// The private-preview access code: sign-up needs the commy_access cookie the
// website sets once the code is entered; sign-in and open servers are untouched.
import assert from "node:assert/strict";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { ACCESS_COOKIE, accessToken, hasAccess } from "../src/access.js";
import { buildAuthOptions } from "../src/auth.js";
import { featureSummary, loadConfig } from "../src/config.js";

const ORIGIN = "http://localhost:3001";

function makeAuth(env = {}) {
  const config = loadConfig({
    BETTER_AUTH_SECRET: "test-secret-".repeat(4),
    DATABASE_URL: "postgres://unused",
    BETTER_AUTH_URL: ORIGIN,
    ...env,
  });
  const options = buildAuthOptions(config, { pool: null, send: async () => {} });
  const db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], doorbellRing: [] };
  options.database = memoryAdapter(db);
  options.rateLimit = { enabled: false };
  return { auth: betterAuth(options), db, config };
}

async function post(auth, path, body, cookie) {
  const headers = { origin: ORIGIN, "content-type": "application/json" };
  if (cookie) headers.cookie = cookie;
  const res = await auth.handler(new Request(`${ORIGIN}/api/auth${path}`, { method: "POST", headers, body: JSON.stringify(body) }));
  const text = await res.text();
  return { status: res.status, json: text ? JSON.parse(text) : null };
}

const account = (email) => ({ email, password: "correct horse battery", name: "Tester" });

test("hasAccess ignores case and spacing in the code and checks the cookie value", () => {
  const cookie = `other=1; ${ACCESS_COOKIE}=${accessToken("  Gabbro ")}`;
  assert.equal(hasAccess(cookie, "gabbro"), true);
  assert.equal(hasAccess(cookie, "basalt"), false);
  assert.equal(hasAccess(`${ACCESS_COOKIE}=nope`, "gabbro"), false);
  assert.equal(hasAccess("", "gabbro"), false);
  assert.equal(hasAccess("", null), true);
});

test("cookie value matches the website's (commy-web lib/access.ts)", () => {
  assert.equal(accessToken(" GABBRO"), "kZ_NWCN_33pptTS8YH6i26MQLSgucw-gjJG9jRRCLAE");
});

test("ACCESS_CODE is normalised and shows in the startup summary", () => {
  const { config } = makeAuth({ ACCESS_CODE: " Gabbro " });
  assert.equal(config.accessCode, "gabbro");
  assert.match(featureSummary(config), /access_code=on/);
  assert.equal(makeAuth().config.accessCode, null);
});

test("without the cookie, sign-up is refused and no user is stored", async () => {
  const { auth, db } = makeAuth({ ACCESS_CODE: "gabbro" });
  const none = await post(auth, "/sign-up/email", account("a@example.com"));
  assert.equal(none.status, 403, JSON.stringify(none.json));
  assert.equal(none.json.code, "ACCESS_CODE_REQUIRED");
  const wrong = await post(auth, "/sign-up/email", account("b@example.com"), `${ACCESS_COOKIE}=${accessToken("basalt")}`);
  assert.equal(wrong.status, 403);
  assert.equal(db.user.length, 0);
});

test("with the cookie, sign-up works, and sign-in never needs it", async () => {
  const { auth, db } = makeAuth({ ACCESS_CODE: "gabbro" });
  const ok = await post(auth, "/sign-up/email", account("c@example.com"), `${ACCESS_COOKIE}=${accessToken("GABBRO")}`);
  assert.equal(ok.status, 200, JSON.stringify(ok.json));
  assert.equal(db.user.length, 1);
  const signIn = await post(auth, "/sign-in/email", { email: "c@example.com", password: "correct horse battery" });
  assert.equal(signIn.status, 200, JSON.stringify(signIn.json));
});

test("without ACCESS_CODE, sign-up is open as before", async () => {
  const { auth } = makeAuth();
  const ok = await post(auth, "/sign-up/email", account("d@example.com"));
  assert.equal(ok.status, 200, JSON.stringify(ok.json));
});
