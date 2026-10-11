import assert from "node:assert/strict";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";
import {
  CODE_ALPHABET,
  CODE_LENGTH,
  clampDays,
  generateCode,
  normalizeCode,
  normalizeServerName,
  validTicket,
} from "../src/invites.js";

const ORIGIN = "http://localhost:3001";
const TICKET = "commy" + "abcdefgh234567".repeat(5);
const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN };

function makeAuth(extraEnv = {}) {
  const options = buildAuthOptions(loadConfig({ ...ENV, ...extraEnv }), { pool: null, send: async () => {} });
  const db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [] };
  options.database = memoryAdapter(db);
  options.rateLimit = { enabled: false };
  const auth = betterAuth(options);
  const call = async (method, path, { body, cookie, query } = {}) => {
    const url = new URL(`${ORIGIN}/api/auth${path}`);
    for (const [k, v] of Object.entries(query ?? {})) url.searchParams.set(k, v);
    // The desktop app sends no Origin header, so neither do these calls unless signed in
    const headers = cookie ? { origin: ORIGIN, cookie } : {};
    if (body) headers["content-type"] = "application/json";
    const res = await auth.handler(new Request(url, { method, headers, body: body ? JSON.stringify(body) : undefined }));
    const text = await res.text();
    return { status: res.status, json: text ? JSON.parse(text) : null, setCookie: res.headers.get("set-cookie") };
  };
  return { db, call };
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

test("tickets must look like Commy invites", () => {
  assert.equal(validTicket(` ${TICKET} `), TICKET);
  assert.equal(validTicket("commyshort"), null);
  assert.equal(validTicket("https://evil.example/" + "a".repeat(50)), null);
  assert.equal(validTicket("commy" + "A".repeat(50)), null);
  assert.equal(validTicket("commy" + "a".repeat(5000)), null);
});

test("server names drop control and direction override characters", () => {
  assert.equal(normalizeServerName("  Gaming‮ Night\n"), "Gaming Night");
  assert.equal(normalizeServerName(""), "a Commy server");
  assert.equal(normalizeServerName("x".repeat(300)).length, 100);
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
  assert.equal(created.json.ticket, undefined, "create never echoes the ticket");
  const days = (new Date(created.json.expiresAt) - Date.now()) / 86400000;
  assert.ok(days > 6.9 && days <= 7, "anonymous links last at most 7 days");

  const again = await call("POST", "/commy/invites/create", { body: { ticket: TICKET, serverName: "Rust Club 2" } });
  assert.equal(again.json.code, created.json.code);
  assert.equal(db.invite.length, 1);

  const resolved = await call("GET", "/commy/invites/resolve", { query: { code: created.json.code.toLowerCase() } });
  assert.equal(resolved.status, 200);
  assert.equal(resolved.json.ticket, TICKET);
  assert.equal(resolved.json.serverName, "Rust Club 2");

  db.invite[0].expiresAt = new Date(Date.now() - 1000);
  assert.equal((await call("GET", "/commy/invites/resolve", { query: { code: created.json.code } })).status, 404);
  assert.equal((await call("GET", "/commy/invites/resolve", { query: { code: "nope" } })).status, 404);
});

test("invalid tickets are refused", async () => {
  const { call } = makeAuth();
  const r = await call("POST", "/commy/invites/create", { body: { ticket: "javascript:alert(1)" } });
  assert.equal(r.status, 400);
});

test("servers can require an account, and only the creator can revoke", async () => {
  const { call } = makeAuth({ INVITES_REQUIRE_ACCOUNT: "true" });
  assert.equal((await call("POST", "/commy/invites/create", { body: { ticket: TICKET } })).status, 401);

  const signUp = async (email) => {
    const r = await call("POST", "/sign-up/email", {
      cookie: "x=1",
      body: { email, password: "correct horse battery", name: "T" },
    });
    assert.equal(r.status, 200, JSON.stringify(r.json));
    return r.setCookie.split(";")[0];
  };
  const alice = await signUp("alice@example.test");
  const bob = await signUp("bob@example.test");

  const created = await call("POST", "/commy/invites/create", { cookie: alice, body: { ticket: TICKET, expiresInDays: 30 } });
  assert.equal(created.status, 200, JSON.stringify(created.json));
  const days = (new Date(created.json.expiresAt) - Date.now()) / 86400000;
  assert.ok(days > 29.9, "signed in users get the longer expiry");

  await call("POST", "/commy/invites/revoke", { cookie: bob, body: { code: created.json.code } });
  assert.equal((await call("GET", "/commy/invites/resolve", { query: { code: created.json.code } })).status, 200);
  await call("POST", "/commy/invites/revoke", { cookie: alice, body: { code: created.json.code } });
  assert.equal((await call("GET", "/commy/invites/resolve", { query: { code: created.json.code } })).status, 404);
});
