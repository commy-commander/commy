import assert from "node:assert/strict";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";
import {
  CODE_ALPHABET,
  CODE_LENGTH,
  clampDays,
  expiryFor,
  NEVER,
  generateCode,
  normalizeCode,
  normalizeName,
  customName,
  hashRevokeSecret,
  MAX_CUSTOM_PER_ACCOUNT,
  isPlainTicket,
  validTicket,
} from "../src/invites.js";

const ORIGIN = "http://localhost:3001";
const TICKET = "sealed1." + "AbCd-_0123456789".repeat(5);
const PLAIN_TICKET = "commy" + "abcdefgh234567".repeat(5);
const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN };

function makeAuth(extraEnv = {}) {
  const options = buildAuthOptions(loadConfig({ ...ENV, ...extraEnv }), { pool: null, send: async () => {} });
  const db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], inviteBackground: [], doorbellRing: [] };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  const auth = betterAuth(options);
  const call = async (method, path, { body, cookie, query } = {}) => {
    const url = new URL(`${ORIGIN}/api/auth${path}`);
    for (const [k, v] of Object.entries(query ?? {})) url.searchParams.set(k, v);
    const headers = cookie ? { origin: ORIGIN, cookie } : {};
    if (body) headers["content-type"] = "application/json";
    const res = await auth.handler(new Request(url, { method, headers, body: body ? JSON.stringify(body) : undefined }));
    const text = await res.text();
    return { status: res.status, json: text ? JSON.parse(text) : null, setCookie: res.headers.get("set-cookie") };
  };
  const signUp = async (email) => {
    const r = await call("POST", "/sign-up/email", {
      cookie: "x=1",
      body: { email, password: "correct horse battery", name: "T" },
    });
    assert.equal(r.status, 200, JSON.stringify(r.json));
    return { cookie: r.setCookie.split(";")[0], id: r.json.user.id };
  };
  return { db, call, signUp };
}

test("codes use an unambiguous alphabet", () => {
  for (let i = 0; i < 200; i++) {
    const code = generateCode();
    assert.equal(code.length, CODE_LENGTH);
    assert.ok([...code].every((c) => CODE_ALPHABET.includes(c)));
    assert.equal(normalizeCode(code.toLowerCase()), code);
  }
  for (const bad of ["", "ABC", "ABCDEFGO", "ABCDEFG1", "ABCDEFGHJ", "../../xx", null]) {
    assert.equal(normalizeCode(bad), null);
  }
});

test("only sealed tickets are accepted", () => {
  assert.equal(validTicket(` ${TICKET} `), TICKET);
  assert.equal(validTicket(PLAIN_TICKET), null, "a plain ticket would reveal the server");
  assert.ok(isPlainTicket(PLAIN_TICKET));
  assert.equal(validTicket("sealed1.short"), null);
  assert.equal(validTicket("sealed1." + "a".repeat(30) + "+/="), null);
  assert.equal(validTicket("https://evil.example/" + "a".repeat(50)), null);
  assert.equal(validTicket("sealed1." + "a".repeat(9000)), null);
});

test("expiry is clamped by account state", () => {
  assert.equal(clampDays(undefined, false), 7);
  assert.equal(clampDays(30, false), 7);
  assert.equal(clampDays(30, true), 30);
  assert.equal(clampDays(90, true), 30);
  assert.equal(clampDays(0, true), 1);
  assert.equal(clampDays(-5, false), 1);
});

test("discovery advertises invite endpoints", () => {
  const d = discoveryDocument(loadConfig({ ...ENV, INVITES_REQUIRE_ACCOUNT: "true" }));
  assert.equal(d.invites.linkPrefix, `${ORIGIN}/i/`);
  assert.equal(d.invites.requireAccount, true);
  assert.equal(d.endpoints.resolveInvite, `${ORIGIN}/api/auth/commy/invites/resolve`);
});

test("anonymous create, dedupe, resolve and expiry", async () => {
  const { db, call } = makeAuth();
  const created = await call("POST", "/commy/invites/create", {
    body: { ticket: TICKET, serverName: "Rust Club", expiresInDays: 30 },
  });
  assert.equal(created.status, 200, JSON.stringify(created.json));
  assert.match(created.json.url, new RegExp(`^${ORIGIN}/i/[${CODE_ALPHABET}]{8}$`));
  assert.equal(created.json.serverName, null, "a server name sent by an old client is ignored");
  assert.equal(created.json.sealed, true);
  assert.equal(db.invite[0].serverName, "", "nothing names the server");
  assert.equal(created.json.ticket, undefined, "create never echoes the ticket");
  const days = (new Date(created.json.expiresAt) - Date.now()) / 86400000;
  assert.ok(days > 6.9 && days <= 7, "anonymous links last at most 7 days");

  const again = await call("POST", "/commy/invites/create", { body: { ticket: TICKET } });
  assert.equal(again.json.code, created.json.code);
  assert.equal(db.invite.length, 1);

  const resolved = await call("GET", "/commy/invites/resolve", { query: { code: created.json.code.toLowerCase() } });
  assert.equal(resolved.status, 200);
  assert.equal(resolved.json.ticket, TICKET);
  assert.equal(resolved.json.serverName, null);

  db.invite[0].expiresAt = new Date(Date.now() - 1000);
  assert.equal((await call("GET", "/commy/invites/resolve", { query: { code: created.json.code } })).status, 404);
  assert.equal((await call("GET", "/commy/invites/resolve", { query: { code: "nope" } })).status, 404);
});

test("expiry choices map to fixed lengths", () => {
  const now = new Date("2026-10-16T12:00:00Z");
  assert.equal(expiryFor("1h", now).toISOString(), "2026-10-16T13:00:00.000Z");
  assert.equal(expiryFor("1d", now).toISOString(), "2026-10-17T12:00:00.000Z");
  assert.equal(expiryFor("30d", now).toISOString(), "2026-11-15T12:00:00.000Z");
  assert.equal(expiryFor("never", now), NEVER);
  assert.equal(expiryFor("90d", now), null);
  assert.equal(expiryFor("__proto__", now), null);
});

test("creators pick the expiry, never for signed in creators, and the row holds nothing else new", async () => {
  const { db, call, signUp } = makeAuth({ INVITES_MAX_PERMANENT: "2" });
  const hour = await call("POST", "/commy/invites/create", { body: { ticket: TICKET, expiresIn: "1h" } });
  assert.equal(hour.status, 200, JSON.stringify(hour.json));
  const mins = (new Date(hour.json.expiresAt) - Date.now()) / 60000;
  assert.ok(mins > 59 && mins <= 60);
  assert.equal(hour.json.permanent, false);

  const month = await call("POST", "/commy/invites/create", { body: { ticket: TICKET + "a", expiresIn: "30d" } });
  assert.ok((new Date(month.json.expiresAt) - Date.now()) / 86400000 > 29.9, "anonymous apps may pick 30 days");

  const anonymous = await call("POST", "/commy/invites/create", { body: { ticket: TICKET + "b", expiresIn: "never" } });
  assert.equal(anonymous.status, 401, "a link that never expires needs an account");
  assert.match(anonymous.json.message, /Sign in/);
  assert.equal(db.invite.length, 2);

  const { cookie, id } = await signUp("forever@example.test");
  const forever = await call("POST", "/commy/invites/create", { cookie, body: { ticket: TICKET + "b", expiresIn: "never" } });
  assert.equal(forever.status, 200, JSON.stringify(forever.json));
  assert.equal(forever.json.expiresAt, null);
  assert.equal(forever.json.permanent, true);
  const row = db.invite.find((r) => r.code === forever.json.code);
  assert.deepEqual(Object.keys(row).sort(), ["code", "createdAt", "createdBy", "expiresAt", "id", "serverName", "ticket"]);
  assert.equal(row.serverName, "");
  assert.equal(row.createdBy, id);
  assert.equal(row.ticket, TICKET + "b", "only the sealed invite, nothing readable");

  await call("POST", "/commy/invites/create", { body: { ticket: TICKET + "c", expiresIn: "1d" } });
  const resolved = await call("GET", "/commy/invites/resolve", { query: { code: forever.json.code } });
  assert.equal(resolved.status, 200);
  assert.equal(resolved.json.expiresAt, null);

  assert.equal((await call("POST", "/commy/invites/create", { cookie, body: { ticket: TICKET + "d", expiresIn: "never" } })).status, 200);
  const full = await call("POST", "/commy/invites/create", { cookie, body: { ticket: TICKET + "e", expiresIn: "never" } });
  assert.equal(full.status, 403, "permanent links are capped");
  assert.equal((await call("POST", "/commy/invites/create", { body: { ticket: TICKET + "f", expiresIn: "2y" } })).status, 400);
});

test("invalid and plain tickets are refused", async () => {
  const { db, call } = makeAuth();
  const r = await call("POST", "/commy/invites/create", { body: { ticket: "javascript:alert(1)" } });
  assert.equal(r.status, 400);
  const plain = await call("POST", "/commy/invites/create", { body: { ticket: PLAIN_TICKET, serverName: "Rust Club" } });
  assert.equal(plain.status, 400);
  assert.match(plain.json.message, /Update Commy/);
  assert.equal(db.invite.length, 0);
});

test("plain invites made before sealing still resolve until they expire", async () => {
  const { db, call } = makeAuth();
  const now = new Date();
  db.invite.push({
    id: "old", code: "AB23CD45", ticket: PLAIN_TICKET, serverName: "Rust Club", createdBy: null,
    createdAt: now, expiresAt: new Date(now.getTime() + 86400000),
  });
  const r = await call("GET", "/commy/invites/resolve", { query: { code: "AB23CD45" } });
  assert.equal(r.status, 200);
  assert.equal(r.json.ticket, PLAIN_TICKET);
  assert.equal(r.json.serverName, "Rust Club");
  assert.equal(r.json.sealed, false);
});

test("servers can require an account, and only the creator can revoke", async () => {
  const { call, signUp } = makeAuth({ INVITES_REQUIRE_ACCOUNT: "true" });
  assert.equal((await call("POST", "/commy/invites/create", { body: { ticket: TICKET } })).status, 401);

  const alice = (await signUp("alice@example.test")).cookie;
  const bob = (await signUp("bob@example.test")).cookie;

  const created = await call("POST", "/commy/invites/create", { cookie: alice, body: { ticket: TICKET, expiresInDays: 30 } });
  assert.equal(created.status, 200, JSON.stringify(created.json));
  const days = (new Date(created.json.expiresAt) - Date.now()) / 86400000;
  assert.ok(days > 29.9, "signed in users get the longer expiry");

  await call("POST", "/commy/invites/revoke", { cookie: bob, body: { code: created.json.code } });
  assert.equal((await call("GET", "/commy/invites/resolve", { query: { code: created.json.code } })).status, 200);
  await call("POST", "/commy/invites/revoke", { cookie: alice, body: { code: created.json.code } });
  assert.equal((await call("GET", "/commy/invites/resolve", { query: { code: created.json.code } })).status, 404);
});

const SECRET = "r".repeat(43);
const BACKGROUND = "sealed1." + "Bg-_0123456789xyz".repeat(10);

test("custom names are short, plain and never look like random codes", () => {
  assert.equal(normalizeName(" Pixel-Pioneers "), "pixel-pioneers");
  assert.equal(customName("team42"), "team42");
  for (const bad of ["ab", "-lead", "trail-", "two--dashes", "has space", "pixelpio", "AB23CD45", "a".repeat(33), "ünïcode", null]) {
    assert.equal(normalizeName(bad), null, String(bad));
  }
  assert.equal(normalizeName("admin"), "admin");
  assert.equal(customName("admin"), null, "reserved names can still be looked up but never claimed");
});

test("signed in creators claim a custom name, keep it, and nobody else can take it", async () => {
  const { db, call, signUp } = makeAuth();
  const body = { ticket: TICKET, expiresIn: "30d", code: "Pixel-Pioneers", revokeHash: hashRevokeSecret(SECRET) };
  assert.equal((await call("POST", "/commy/invites/create", { body })).status, 401, "a custom name needs an account");

  const alice = await signUp("alice@example.test");
  const bob = await signUp("bob@example.test");
  const created = await call("POST", "/commy/invites/create", { cookie: alice.cookie, body });
  assert.equal(created.status, 200, JSON.stringify(created.json));
  assert.equal(created.json.url, `${ORIGIN}/i/pixel-pioneers`);
  assert.equal(db.invite[0].custom, true);

  const resolved = await call("GET", "/commy/invites/resolve", { query: { code: "PIXEL-pioneers" } });
  assert.equal(resolved.status, 200);
  assert.equal(resolved.json.ticket, TICKET);
  assert.equal(resolved.json.hasBackground, false);

  const stolen = await call("POST", "/commy/invites/create", { cookie: bob.cookie, body: { ...body, ticket: TICKET + "x" } });
  assert.equal(stolen.status, 409);
  const moved = await call("POST", "/commy/invites/create", { cookie: alice.cookie, body: { ...body, ticket: TICKET + "y", expiresIn: "1d" } });
  assert.equal(moved.status, 200);
  assert.equal(db.invite.length, 1, "the owner points the same name at a new invite");
  assert.equal(db.invite[0].ticket, TICKET + "y");

  for (const code of ["admin", "AB23CD45", "x"]) {
    assert.equal((await call("POST", "/commy/invites/create", { cookie: alice.cookie, body: { ...body, code } })).status, 400, code);
  }
  for (let i = 1; i < MAX_CUSTOM_PER_ACCOUNT; i++) {
    const r = await call("POST", "/commy/invites/create", { cookie: alice.cookie, body: { ...body, ticket: `${TICKET}${i}`, code: `name-${i}` } });
    assert.equal(r.status, 200, JSON.stringify(r.json));
  }
  const over = await call("POST", "/commy/invites/create", { cookie: alice.cookie, body: { ...body, code: "one-too-many" } });
  assert.equal(over.status, 403);

  db.invite.find((r) => r.code === "pixel-pioneers").expiresAt = new Date(Date.now() - 1000);
  const after = await call("POST", "/commy/invites/create", { cookie: bob.cookie, body: { ...body, ticket: TICKET + "z" } });
  assert.equal(after.status, 200, "an expired name is free again");
});

test("whoever holds the revoke secret can revoke, without an account", async () => {
  const { db, call } = makeAuth();
  const created = await call("POST", "/commy/invites/create", { body: { ticket: TICKET, expiresIn: "7d", revokeHash: hashRevokeSecret(SECRET) } });
  assert.equal(created.status, 200, JSON.stringify(created.json));
  assert.equal(db.invite[0].revokeHash, hashRevokeSecret(SECRET), "only a hash of the secret is kept");
  assert.equal((await call("POST", "/commy/invites/create", { body: { ticket: TICKET + "a", revokeHash: "nope" } })).status, 400);

  const wrong = await call("POST", "/commy/invites/revoke", { body: { code: created.json.code, secret: "s".repeat(43) } });
  assert.equal(wrong.status, 200, "a wrong secret looks the same as a right one");
  assert.equal((await call("GET", "/commy/invites/resolve", { query: { code: created.json.code } })).status, 200);
  await call("POST", "/commy/invites/revoke", { body: { code: created.json.code, secret: SECRET } });
  assert.equal((await call("GET", "/commy/invites/resolve", { query: { code: created.json.code } })).status, 404);

  const old = await call("POST", "/commy/invites/create", { body: { ticket: TICKET + "b" } });
  await call("POST", "/commy/invites/revoke", { body: { code: old.json.code, secret: SECRET } });
  assert.equal((await call("GET", "/commy/invites/resolve", { query: { code: old.json.code } })).status, 200, "links made without a hash need their creator");
});

test("an invite background is stored sealed, needs an account and goes with its link", async () => {
  const { db, call, signUp } = makeAuth();
  const body = { ticket: TICKET, expiresIn: "7d", background: BACKGROUND, revokeHash: hashRevokeSecret(SECRET) };
  assert.equal((await call("POST", "/commy/invites/create", { body })).status, 401);
  const { cookie } = await signUp("bg@example.test");
  assert.equal((await call("POST", "/commy/invites/create", { cookie, body: { ...body, background: "data:image/png;base64,AAAA" } })).status, 400);
  assert.equal((await call("POST", "/commy/invites/create", { cookie, body: { ...body, background: "sealed1." + "a".repeat(700000) } })).status, 400);

  const created = await call("POST", "/commy/invites/create", { cookie, body });
  assert.equal(created.status, 200, JSON.stringify(created.json));
  assert.deepEqual(Object.keys(db.inviteBackground[0]).sort(), ["code", "data", "expiresAt", "id"]);
  const resolved = await call("GET", "/commy/invites/resolve", { query: { code: created.json.code } });
  assert.equal(resolved.json.hasBackground, true);
  const fetched = await call("GET", "/commy/invites/background", { query: { code: created.json.code } });
  assert.equal(fetched.status, 200);
  assert.equal(fetched.json.background, BACKGROUND);

  const plain = await call("POST", "/commy/invites/create", { body: { ticket: TICKET + "p" } });
  assert.equal((await call("GET", "/commy/invites/background", { query: { code: plain.json.code } })).status, 404);

  await call("POST", "/commy/invites/revoke", { body: { code: created.json.code, secret: SECRET } });
  assert.equal(db.inviteBackground.length, 0, "revoking drops the background too");
  assert.equal((await call("GET", "/commy/invites/background", { query: { code: created.json.code } })).status, 404);

  const again = await call("POST", "/commy/invites/create", { cookie, body: { ...body, ticket: TICKET + "q" } });
  db.invite.find((r) => r.code === again.json.code).expiresAt = new Date(Date.now() - 1000);
  db.inviteBackground[0].expiresAt = new Date(Date.now() - 1000);
  assert.equal((await call("GET", "/commy/invites/background", { query: { code: again.json.code } })).status, 404);
  await call("POST", "/commy/invites/create", { body: { ticket: TICKET + "r" } });
  assert.equal(db.inviteBackground.length, 0, "expired backgrounds are swept");
});
