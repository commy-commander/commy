import assert from "node:assert/strict";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { cleanAccountImage, cleanAccountName, NAME_MAX_CHARS } from "../src/account-profile.js";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { loadConfig } from "../src/config.js";

const ORIGIN = "http://localhost:3001";

function makeAuth() {
  const config = loadConfig({ BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN });
  const options = buildAuthOptions(config, { pool: null, send: async () => {} });
  const db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], inviteBackground: [], doorbellRing: [] };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  return betterAuth(options);
}

async function post(auth, path, body, cookie) {
  const headers = { origin: ORIGIN, "content-type": "application/json", ...(cookie ? { cookie } : {}) };
  const res = await auth.handler(new Request(`${ORIGIN}/api/auth${path}`, { method: "POST", headers, body: JSON.stringify(body) }));
  return { res, json: await res.json() };
}

async function sessionUser(auth, cookie) {
  const res = await auth.handler(new Request(`${ORIGIN}/api/auth/get-session`, { headers: { origin: ORIGIN, cookie } }));
  return (await res.json()).user;
}

const cookiesFrom = (res) => res.headers.getSetCookie().map((c) => c.split(";")[0]).join("; ");

const cp = (...codePoints) => String.fromCodePoint(...codePoints);
const ZWSP = cp(0x200b);
const ZWJ = cp(0x200d);

test("account names keep what people can see and drop what they cannot", () => {
  assert.equal(cleanAccountName("  Ada \n\t Lovelace  "), "Ada Lovelace");
  assert.equal(cleanAccountName(`ad${ZWSP}min${cp(0x202e, 0x2066, 0x0000, 0x007f, 0x00ad)}`), "admin");
  assert.equal(cleanAccountName(cp(0x3164, 0x115f)), "");
  assert.equal(cleanAccountName(`Cafe${cp(0x301)}`), `Caf${cp(0xe9)}`);
  assert.equal(cleanAccountName(`q${cp(0x301, 0x302, 0x303, 0x304, 0x305, 0x306)}`), `q${cp(0x301, 0x302, 0x303)}`);
  const emoji = `${cp(0x1f469)}${ZWJ}${cp(0x1f4bb)} Dev ${cp(0x1f3f4, 0xe0067, 0xe0062, 0xe0073, 0xe0063, 0xe0074, 0xe007f)}`;
  assert.equal(cleanAccountName(emoji), emoji);
  assert.equal(cleanAccountName("<script>alert(1)</script>"), "<script>alert(1)</script>");
  assert.equal(cleanAccountName(null), "");
  assert.equal(cleanAccountName({ toString: () => "x" }), "");
});

test("account names stop at the limit without splitting a character", () => {
  assert.equal(cleanAccountName("a".repeat(10_000)), "a".repeat(NAME_MAX_CHARS));
  const family = `${cp(0x1f468)}${ZWJ}${cp(0x1f469)}${ZWJ}${cp(0x1f467)}`;
  const name = cleanAccountName(`${"b".repeat(NAME_MAX_CHARS - 2)}${family}`);
  assert.equal(name, "b".repeat(NAME_MAX_CHARS - 2));
});

test("account pictures are https links or nothing", () => {
  assert.equal(cleanAccountImage("https://cdn.example.com/a.png"), "https://cdn.example.com/a.png");
  assert.equal(cleanAccountImage("javascript:alert(1)"), null);
  assert.equal(cleanAccountImage("data:image/svg+xml,<svg onload=alert(1)>"), null);
  assert.equal(cleanAccountImage("http://cdn.example.com/a.png"), null);
  assert.equal(cleanAccountImage("https://user:pass@cdn.example.com/a.png"), null);
  assert.equal(cleanAccountImage(`https://cdn.example.com/${"a".repeat(3000)}`), null);
  assert.equal(cleanAccountImage("not a url"), null);
});

test("sign-up and profile updates store the cleaned name and picture", async () => {
  const auth = makeAuth();
  const signUp = await post(auth, "/sign-up/email", {
    email: "mallory@example.com",
    password: "correct horse battery",
    name: `${cp(0x202e)}Mal${ZWSP}lory${"!".repeat(5000)}`,
    image: "javascript:alert(1)",
  });
  assert.equal(signUp.res.status, 200);
  const cookie = cookiesFrom(signUp.res);
  const created = await sessionUser(auth, cookie);
  assert.equal(created.name, `Mallory${"!".repeat(NAME_MAX_CHARS - 7)}`);
  assert.equal(created.image, null);

  const update = await post(auth, "/update-user", { name: cp(0x0000, 0x3164), image: "https://cdn.example.com/me.png" }, cookie);
  assert.equal(update.res.status, 200);
  const updated = await sessionUser(auth, cookie);
  assert.equal(updated.name, "");
  assert.equal(updated.image, "https://cdn.example.com/me.png");
});

test("a sign-up whose name is only hidden characters is named after the email", async () => {
  const auth = makeAuth();
  const signUp = await post(auth, "/sign-up/email", { email: "quiet.one@example.com", password: "correct horse battery", name: `${ZWSP}${ZWSP}` });
  assert.equal(signUp.res.status, 200);
  assert.equal((await sessionUser(auth, cookiesFrom(signUp.res))).name, "quiet.one");
});
