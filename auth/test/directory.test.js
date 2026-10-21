import assert from "node:assert/strict";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";
import {
  CATEGORIES,
  DESCRIPTION_MAX,
  MAX_PER_ACCOUNT,
  MEMBER_BUCKETS,
  NAME_MAX,
  PAGE_SIZE,
  cleanDescription,
  cleanName,
  matchesWords,
  parseCursor,
  publicListing,
  searchWords,
  sortListings,
  validBucket,
  validCategory,
  validKey,
} from "../src/directory.js";

const ORIGIN = "http://localhost:3001";
const TICKET = "sealed1." + "AbCd-_0123456789".repeat(5);
const KEY = "k".repeat(43);
const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN };

function makeAuth(extraEnv = {}) {
  const options = buildAuthOptions(loadConfig({ ...ENV, ...extraEnv }), { pool: null, send: async () => {} });
  const db = {
    user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [],
    invite: [], inviteBackground: [], doorbellRing: [], directoryListing: [],
  };
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
  const permanentInvite = async (cookie, suffix = "") => {
    const r = await call("POST", "/commy/invites/create", { cookie, body: { ticket: TICKET + suffix, expiresIn: "never" } });
    assert.equal(r.status, 200, JSON.stringify(r.json));
    return r.json.code;
  };
  return { db, call, signUp, permanentInvite };
}

const listing = (code, extra = {}) => ({ code, key: KEY, name: "Rust Hangout", description: "Borrow checker support group", category: "tech", members: 25, ...extra });

test("names and descriptions are tidied and bounded", () => {
  assert.equal(cleanName("  Rust   Hangout "), "Rust Hangout");
  assert.equal(cleanName("a\u0000b"), "a b");
  assert.equal(cleanName(""), null);
  assert.equal(cleanName("x".repeat(NAME_MAX + 1)), null);
  assert.equal(cleanDescription(""), "");
  assert.equal(cleanDescription("  two  spaces\n\n\n\nmany lines "), "two spaces\n\nmany lines");
  assert.equal(cleanDescription("x".repeat(DESCRIPTION_MAX)), "x".repeat(DESCRIPTION_MAX));
  assert.equal(cleanDescription("x".repeat(DESCRIPTION_MAX + 1)), null);
  assert.equal(cleanDescription("bell\u0007"), null);
});

test("categories, buckets and keys come from fixed sets", () => {
  for (const c of CATEGORIES) assert.equal(validCategory(c.toUpperCase()), c);
  assert.equal(validCategory("crypto"), null);
  assert.equal(validCategory("__proto__"), null);
  for (const b of MEMBER_BUCKETS) assert.equal(validBucket(b), b);
  assert.equal(validBucket(37), null);
  assert.equal(validBucket("25"), null);
  assert.equal(validKey(KEY), KEY);
  assert.equal(validKey("Zx9-_".repeat(4) + "ab"), "Zx9-_".repeat(4) + "ab");
  assert.equal(validKey("short"), null);
  assert.equal(validKey("k".repeat(43) + "!"), null);
});

test("search words are normalized and matched against name and description", () => {
  assert.deepEqual(searchWords("  Rust   HELP "), ["rust", "help"]);
  assert.deepEqual(searchWords(""), []);
  assert.deepEqual(searchWords(undefined), []);
  assert.equal(searchWords("a b c d e f g h").length, 6);
  assert.equal(searchWords("x".repeat(200))[0].length, 80);
  const row = { name: "Rust Hangout", description: "Borrow checker support group" };
  assert.equal(matchesWords(row, ["rust", "borrow"]), true);
  assert.equal(matchesWords(row, ["rust", "python"]), false);
  assert.equal(matchesWords(row, []), true);
});

test("cursors are offsets and sorting puts bigger, fresher servers first", () => {
  assert.equal(parseCursor(undefined), 0);
  assert.equal(parseCursor(""), 0);
  assert.equal(parseCursor("48"), 48);
  assert.equal(parseCursor("-1"), null);
  assert.equal(parseCursor("abc"), null);
  assert.equal(parseCursor("1".repeat(8)), null);
  const old = new Date("2026-01-01T00:00:00Z");
  const fresh = new Date("2026-06-01T00:00:00Z");
  const sorted = sortListings([
    { code: "B", members: 25, updatedAt: old },
    { code: "A", members: 25, updatedAt: fresh },
    { code: "C", members: 500, updatedAt: old },
    { code: "D", members: 25, updatedAt: fresh },
  ]);
  assert.deepEqual(sorted.map((r) => r.code), ["C", "A", "D", "B"]);
  assert.deepEqual(Object.keys(publicListing({ ...listing("X"), id: "1", createdBy: "u", createdAt: old, updatedAt: old })).sort(),
    ["category", "code", "description", "key", "members", "name"]);
});

test("discovery advertises the directory endpoints", () => {
  const d = discoveryDocument(loadConfig(ENV));
  assert.equal(d.endpoints.directoryList, `${ORIGIN}/api/auth/commy/directory/list`);
  assert.equal(d.endpoints.directoryPublish, `${ORIGIN}/api/auth/commy/directory/publish`);
});

test("only the signed in owner of a permanent invite can list it, and the row holds no more than the listing", async () => {
  const { db, call, signUp, permanentInvite } = makeAuth();
  assert.equal((await call("POST", "/commy/directory/publish", { body: listing("ABCDEFGH") })).status, 401);

  const alice = await signUp("alice@example.test");
  const bob = await signUp("bob@example.test");
  const code = await permanentInvite(alice.cookie);

  const stolen = await call("POST", "/commy/directory/publish", { cookie: bob.cookie, body: listing(code) });
  assert.equal(stolen.status, 404, "another account's invite is as good as missing");
  assert.equal((await call("POST", "/commy/directory/publish", { cookie: alice.cookie, body: listing("ZZZZ2222") })).status, 404);

  const temporary = await call("POST", "/commy/invites/create", { cookie: alice.cookie, body: { ticket: TICKET + "t", expiresIn: "7d" } });
  const short = await call("POST", "/commy/directory/publish", { cookie: alice.cookie, body: listing(temporary.json.code) });
  assert.equal(short.status, 400);
  assert.match(short.json.message, /never expires/);

  const created = await call("POST", "/commy/directory/publish", { cookie: alice.cookie, body: listing(code.toLowerCase()) });
  assert.equal(created.status, 200, JSON.stringify(created.json));
  assert.deepEqual(created.json, { code, key: KEY, name: "Rust Hangout", description: "Borrow checker support group", category: "tech", members: 25 });
  assert.deepEqual(Object.keys(db.directoryListing[0]).sort(),
    ["category", "code", "createdAt", "createdBy", "description", "id", "key", "members", "name", "updatedAt"]);
  assert.equal(db.directoryListing[0].createdBy, alice.id);

  const updated = await call("POST", "/commy/directory/publish", {
    cookie: alice.cookie, body: listing(code, { description: "Now with more crabs", category: "gaming", members: 50 }),
  });
  assert.equal(updated.status, 200);
  assert.equal(db.directoryListing.length, 1, "publishing again updates the same listing");
  assert.equal(db.directoryListing[0].description, "Now with more crabs");
  assert.equal(db.directoryListing[0].category, "gaming");
  assert.equal(db.directoryListing[0].members, 50);
});

test("listing fields are validated", async () => {
  const { call, signUp, permanentInvite } = makeAuth();
  const alice = await signUp("alice@example.test");
  const code = await permanentInvite(alice.cookie);
  const publish = (extra) => call("POST", "/commy/directory/publish", { cookie: alice.cookie, body: listing(code, extra) });
  assert.equal((await publish({ name: "" })).status, 400);
  assert.equal((await publish({ name: "x".repeat(NAME_MAX + 1) })).status, 400);
  assert.equal((await publish({ description: "x".repeat(DESCRIPTION_MAX + 1) })).status, 400);
  assert.equal((await publish({ category: "crypto" })).status, 400);
  assert.equal((await publish({ members: 37 })).status, 400);
  assert.equal((await publish({ key: "nope" })).status, 400);
  assert.equal((await publish({ code: "../x" })).status, 400);
  assert.equal((await publish({ description: undefined })).status, 200, "the description may be left out");
});

test("each account lists at most a few servers", async () => {
  const { call, signUp, permanentInvite } = makeAuth();
  const alice = await signUp("alice@example.test");
  for (let i = 0; i < MAX_PER_ACCOUNT; i++) {
    const code = await permanentInvite(alice.cookie, String(i));
    const r = await call("POST", "/commy/directory/publish", { cookie: alice.cookie, body: listing(code) });
    assert.equal(r.status, 200, JSON.stringify(r.json));
  }
  const extra = await permanentInvite(alice.cookie, "extra");
  const over = await call("POST", "/commy/directory/publish", { cookie: alice.cookie, body: listing(extra) });
  assert.equal(over.status, 403);
  assert.match(over.json.message, /Unlist one first/);
});

test("anyone can browse, results hide the creator, and only the owner can unlist", async () => {
  const { db, call, signUp, permanentInvite } = makeAuth();
  const alice = await signUp("alice@example.test");
  const bob = await signUp("bob@example.test");
  const code = await permanentInvite(alice.cookie);
  await call("POST", "/commy/directory/publish", { cookie: alice.cookie, body: listing(code) });
  const bobCode = await permanentInvite(bob.cookie, "b");
  await call("POST", "/commy/directory/publish", {
    cookie: bob.cookie, body: listing(bobCode, { name: "Synth Lounge", description: "Modular jams every Friday", category: "music", members: 100 }),
  });

  const all = await call("GET", "/commy/directory/list");
  assert.equal(all.status, 200);
  assert.deepEqual(all.json.listings.map((l) => l.name), ["Synth Lounge", "Rust Hangout"], "bigger servers come first");
  assert.equal(all.json.nextCursor, null);
  for (const row of all.json.listings) {
    assert.deepEqual(Object.keys(row).sort(), ["category", "code", "description", "key", "members", "name"]);
  }

  const music = await call("GET", "/commy/directory/list", { query: { category: "music" } });
  assert.deepEqual(music.json.listings.map((l) => l.code), [bobCode]);
  assert.equal((await call("GET", "/commy/directory/list", { query: { category: "crypto" } })).status, 400);

  const words = await call("GET", "/commy/directory/list", { query: { q: "BORROW rust" } });
  assert.deepEqual(words.json.listings.map((l) => l.code), [code]);
  assert.deepEqual((await call("GET", "/commy/directory/list", { query: { q: "borrow modular" } })).json.listings, []);
  assert.equal((await call("GET", "/commy/directory/list", { query: { cursor: "x" } })).status, 400);

  assert.equal((await call("POST", "/commy/directory/unlist", { body: { code } })).status, 401);
  await call("POST", "/commy/directory/unlist", { cookie: bob.cookie, body: { code } });
  assert.equal(db.directoryListing.length, 2, "someone else's unlist changes nothing");
  const own = await call("POST", "/commy/directory/unlist", { cookie: alice.cookie, body: { code } });
  assert.equal(own.status, 200);
  assert.equal(db.directoryListing.length, 1);
  assert.equal((await call("POST", "/commy/directory/unlist", { cookie: alice.cookie, body: { code } })).status, 200, "unlisting twice is fine");
});

test("listings whose invite was revoked disappear from the directory", async () => {
  const { db, call, signUp, permanentInvite } = makeAuth();
  const alice = await signUp("alice@example.test");
  const code = await permanentInvite(alice.cookie);
  await call("POST", "/commy/directory/publish", { cookie: alice.cookie, body: listing(code) });
  await call("POST", "/commy/invites/revoke", { cookie: alice.cookie, body: { code } });
  const page = await call("GET", "/commy/directory/list");
  assert.deepEqual(page.json.listings, []);
  assert.equal(db.directoryListing.length, 0, "the stale listing is swept");
});

test("pages hold a fixed number of listings and the cursor walks them in order", async () => {
  const { db, call } = makeAuth();
  const now = new Date();
  const total = PAGE_SIZE + 5;
  for (let i = 0; i < total; i++) {
    const code = `PAGE${"ABCDEFGHJK"[Math.floor(i / 10)]}${"ABCDEFGHJK"[i % 10]}22`;
    db.invite.push({ id: `i${i}`, code, ticket: TICKET, serverName: "", createdBy: "u", createdAt: now, expiresAt: new Date("9999-12-31T23:59:59.000Z") });
    db.directoryListing.push({
      id: `l${i}`, code, key: KEY, name: `Server ${i}`, description: "", category: "other",
      members: 10, createdBy: "u", createdAt: now, updatedAt: new Date(now.getTime() - i * 1000),
    });
  }
  const first = await call("GET", "/commy/directory/list");
  assert.equal(first.json.listings.length, PAGE_SIZE);
  assert.equal(first.json.listings[0].name, "Server 0", "fresher listings come first within a size");
  assert.equal(first.json.nextCursor, String(PAGE_SIZE));
  const second = await call("GET", "/commy/directory/list", { query: { cursor: first.json.nextCursor } });
  assert.equal(second.json.listings.length, total - PAGE_SIZE);
  assert.equal(second.json.nextCursor, null);
  const seen = new Set([...first.json.listings, ...second.json.listings].map((l) => l.code));
  assert.equal(seen.size, total, "no listing is repeated or skipped across pages");
});
