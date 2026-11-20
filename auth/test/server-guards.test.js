import assert from "node:assert/strict";
import { createPrivateKey, createPublicKey, generateKeyPairSync, sign, verify } from "node:crypto";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";
import { cardPayload, checkCardRequest, issueCard, requestMessage } from "../src/home-card.js";
import { createHomeKeys, isHomeOrigin, MISSING_FOR_MS, isPublicAddress, publishedKeyOf, syncUrlOf } from "../src/home-keys.js";
import { blockedServerLine, blockedServers, createServerBlocklist, LIST_PURPOSE } from "../src/server-blocklist.js";
import { keyFromSeed, keyId, loadListKey, loadServerKey, signedBytes } from "../src/server-key.js";

const ORIGIN = "http://localhost:3001";
const SECRET = "test-secret-".repeat(4);
const ENV = { BETTER_AUTH_SECRET: SECRET, DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN };
const SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
const PKCS8_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const USER = "6f1c1d6e-2b7a-4c55-9d4e-0d8a1b2c3d4e";
const quiet = { log: () => {}, warn: () => {} };

function publicKey(hex) {
  return createPublicKey({ key: Buffer.concat([SPKI_PREFIX, Buffer.from(hex, "hex")]), format: "der", type: "spki" });
}

function identity() {
  const seed = Buffer.alloc(32, 9);
  const privateKey = createPrivateKey({ key: Buffer.concat([PKCS8_PREFIX, seed]), format: "der", type: "pkcs8" });
  const hex = createPublicKey(privateKey).export({ format: "der", type: "spki" }).subarray(12).toString("hex");
  return { hex, sign: (message) => sign(null, Buffer.from(message), privateKey).toString("hex") };
}

test("the server key comes from the secret unless one is set, and differs from the list key", () => {
  const config = loadConfig(ENV);
  const again = loadConfig(ENV);
  assert.equal(config.serverKey.published.public, again.serverKey.published.public);
  assert.notEqual(config.serverKey.published.public, config.listKey.published.public);
  const other = loadConfig({ ...ENV, BETTER_AUTH_SECRET: "other-secret-".repeat(4) });
  assert.notEqual(config.serverKey.published.public, other.serverKey.published.public);
  const set = loadConfig({ ...ENV, AUTH_SERVER_KEY: "03".repeat(32) });
  assert.equal(set.serverKey.published.public, keyFromSeed(Buffer.alloc(32, 3)).published.public);
  assert.throws(() => loadConfig({ ...ENV, COMMY_LIST_KEY: "nothex" }), /64 hex/);
});

test("signatures match the app's server key format", () => {
  const key = keyFromSeed(Buffer.alloc(32, 3));
  assert.equal(key.published.public, "ed4928c628d1c2c6eae90338905995612959273a5c63f93636c14614ac8737d1"); // gitleaks:allow
  assert.equal(key.published.id, keyId(key.published.public));
  assert.equal(key.published.id.length, 16);
  const sig = key.sign("home-card", "https://commy.chat", "x");
  assert.equal(signedBytes("home-card", "https://commy.chat", "x").toString(), "commy-server/v1\nhome-card\nhttps://commy.chat\nx");
  assert.ok(verify(null, signedBytes("home-card", "https://commy.chat", "x"), publicKey(key.published.public), Buffer.from(sig, "hex")));
  assert.equal(sig, key.sign("home-card", "https://commy.chat", "x"), "ed25519 signatures are deterministic");
});

test("discovery publishes the server key, the blocklist and the home card endpoints", () => {
  const config = loadConfig(ENV);
  const doc = discoveryDocument(config);
  assert.deepEqual(doc.serverKey, config.serverKey.published);
  assert.equal(doc.serverBlocklist.url, `${ORIGIN}/api/auth/commy/server-blocklist`);
  assert.deepEqual(doc.serverBlocklist.key, config.listKey.published);
  assert.equal(doc.endpoints.homeCard, `${ORIGIN}/api/auth/commy/home-card`);
  assert.equal(doc.endpoints.homeKey, `${ORIGIN}/api/auth/commy/home-key`);
});

test("a home card names the person and is signed by the server key", () => {
  const key = keyFromSeed(Buffer.alloc(32, 3));
  const person = identity();
  const card = issueCard(key, "https://commy.chat", USER, person.hex, 1_000);
  assert.equal(card.key, key.published.public);
  assert.equal(card.expires - card.issued, 30 * 24 * 60 * 60 * 1000);
  const bytes = signedBytes("home-card", "https://commy.chat", cardPayload(USER, person.hex, card.issued, card.expires));
  assert.ok(verify(null, bytes, publicKey(key.published.public), Buffer.from(card.sig, "hex")));
});

test("a card request must be signed by the identity it names and be recent", () => {
  const person = identity();
  const home = "https://commy.chat";
  const now = 5_000_000;
  const request = { user: USER, identity: person.hex, at: now, proof: person.sign(requestMessage(home, USER, person.hex, now)) };
  assert.equal(checkCardRequest(home, request, now), null);
  assert.match(checkCardRequest("https://cornmy.chat", request, now), /did not sign/);
  assert.match(checkCardRequest(home, { ...request, user: "nope" }, now), /user id/);
  assert.match(checkCardRequest(home, request, now + 11 * 60 * 1000), /too old/);
  const stranger = generateKeyPairSync("ed25519").publicKey.export({ format: "der", type: "spki" }).subarray(12).toString("hex");
  assert.match(checkCardRequest(home, { ...request, identity: stranger }, now), /did not sign/);
});

test("a signed-in app gets a home card, a signed-out one does not", async () => {
  const options = buildAuthOptions(loadConfig(ENV), { pool: null, send: async () => {} });
  options.database = sealedAtRest(memoryAdapter({ user: [], session: [], commyIdentity: [], account: [], verification: [], deviceCode: [], linkedDevice: [], commyMove: [] }), options.secret);
  options.rateLimit = { enabled: false };
  const auth = betterAuth(options);
  const call = async (path, { body, cookie } = {}) => {
    const headers = { "content-type": "application/json", ...(cookie ? { origin: ORIGIN, cookie } : {}) };
    const res = await auth.handler(new Request(`${ORIGIN}/api/auth${path}`, { method: "POST", headers, body: JSON.stringify(body) }));
    const text = await res.text();
    return { status: res.status, json: text ? JSON.parse(text) : null, setCookie: res.headers.get("set-cookie") };
  };
  const person = identity();
  const at = Date.now();
  const body = { user: USER, identity: person.hex, at, proof: person.sign(requestMessage(ORIGIN, USER, person.hex, at)) };
  assert.equal((await call("/commy/home-card", { body })).status, 401);
  const signUp = await call("/sign-up/email", { cookie: "x=1", body: { email: "a@example.org", password: "correct horse battery", name: "A" } });
  const cookie = signUp.setCookie.split(";")[0];
  const issued = await call("/commy/home-card", { body, cookie });
  assert.equal(issued.status, 200, JSON.stringify(issued.json));
  assert.equal(issued.json.card.home, ORIGIN);
  assert.equal(issued.json.card.user, USER);
  const forged = await call("/commy/home-card", { body: { ...body, proof: "00".repeat(64) }, cookie });
  assert.equal(forged.status, 400);
});

test("blocklist lines take a domain, an optional date and a comment", () => {
  assert.deepEqual(blockedServerLine("Spam.Example  2026-11-15  # flooded a community"), { domain: "spam.example", since: Date.parse("2026-11-15T00:00:00Z") });
  assert.deepEqual(blockedServerLine("https://bad.example/"), { domain: "bad.example", since: 0 });
  for (const line of ["", "# note", "localhost", "10.0.0.1", "bad"]) assert.equal(blockedServerLine(line), null, line);
  assert.deepEqual(blockedServers("b.example\na.example\nb.example 2026-01-01\n").map((e) => e.domain), ["a.example", "b.example"]);
});

test("the served blocklist is signed by the list key over its own origin", async () => {
  const config = loadConfig(ENV);
  const list = createServerBlocklist({ listKey: config.listKey, origin: "https://commy.chat", serverKey: config.serverKey, log: quiet, now: () => 1_000 });
  await list.refresh();
  const sent = {};
  list.respond({ method: "GET", headers: {} }, { writeHead: (status, headers) => Object.assign(sent, { status, headers }), end: (body) => (sent.body = body) });
  assert.equal(sent.status, 200);
  const envelope = JSON.parse(sent.body);
  assert.equal(envelope.key, config.listKey.published.id);
  const bytes = signedBytes(LIST_PURPOSE, "https://commy.chat", envelope.list);
  assert.ok(verify(null, bytes, publicKey(config.listKey.published.public), Buffer.from(envelope.sig, "hex")));
  const body = JSON.parse(envelope.list);
  assert.deepEqual(body.domains, []);
  assert.deepEqual(body.homes, [{ home: "https://commy.chat", key: config.serverKey.published }]);
  assert.equal(body.v, 1);
  const again = {};
  list.respond({ method: "GET", headers: { "if-none-match": sent.headers.etag } }, { writeHead: (status) => (again.status = status), end: () => {} });
  assert.equal(again.status, 304);
});

test("the blocklist follows its file and blocks subdomains", async (t) => {
  const { mkdtemp, writeFile } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const dir = await mkdtemp(join(tmpdir(), "blocklist-"));
  const file = join(dir, "servers.txt");
  await writeFile(file, "spam.example # drill\n");
  const config = loadConfig(ENV);
  const list = createServerBlocklist({ listKey: config.listKey, origin: ORIGIN, serverKey: config.serverKey, file, log: quiet });
  await list.refresh();
  assert.ok(list.blocks("spam.example") && list.blocks("sync.spam.example"));
  assert.ok(!list.blocks("notspam.example"));
  await writeFile(file, "spam.example\nmore.example\n");
  await list.refresh();
  assert.ok(list.blocks("more.example"));
  t.after(() => list.stop());
});

test("home keys are fetched only for public https homes and remembered", async () => {
  assert.ok(isHomeOrigin("https://commy.chat"));
  for (const bad of ["http://commy.chat", "https://localhost", "https://10.0.0.1", "https://box.local", "https://commy.chat/path", "https://a.example"]) {
    assert.ok(!isHomeOrigin(bad), bad);
  }
  for (const address of ["10.1.2.3", "127.0.0.1", "169.254.169.254", "::1", "fd00::1", "::ffff:10.0.0.1", "100.64.0.1"]) {
    assert.ok(!isPublicAddress(address), address);
  }
  assert.ok(isPublicAddress("1.1.1.1") && isPublicAddress("2606:4700::1111"));
  const published = keyFromSeed(Buffer.alloc(32, 4)).published;
  let fetched = 0;
  const homeKeys = createHomeKeys({
    ownHome: "https://commy.chat",
    ownKey: { id: "own" },
    fetchDocument: async (home) => {
      fetched += 1;
      return home === "https://friend.org" ? { issuer: home, serverKey: published } : { issuer: "https://elsewhere.org", serverKey: published };
    },
  });
  assert.deepEqual(await homeKeys.keyOf("https://commy.chat"), { id: "own" });
  assert.deepEqual(await homeKeys.keyOf("https://friend.org"), published);
  assert.deepEqual(await homeKeys.keyOf("https://friend.org"), published);
  assert.equal(fetched, 1);
  assert.equal(await homeKeys.keyOf("https://liar.org"), null, "a document for another issuer is not believed");
  assert.equal(publishedKeyOf({ issuer: "https://x.org", serverKey: { ...published, id: "0000000000000000" } }, "https://x.org"), null);
});

test("a home's sync address is learned only from its own discovery document", async () => {
  const home = "https://friend.org";
  const doc = (sync, issuer = home) => ({ issuer, endpoints: { sync } });
  assert.equal(syncUrlOf(doc("wss://friend.org/sync"), home), "wss://friend.org/sync");
  assert.equal(syncUrlOf(doc("wss://friend.org/sync/"), home), "wss://friend.org/sync");
  assert.equal(syncUrlOf(doc("wss://commy.chat/sync"), home), null, "a home cannot point at another site's sync");
  assert.equal(syncUrlOf(doc("ws://friend.org/sync"), home), null);
  assert.equal(syncUrlOf(doc("wss://user:pw@friend.org/sync"), home), null);
  assert.equal(syncUrlOf(doc("wss://friend.org/sync", "https://elsewhere.org"), home), null);
  assert.equal(syncUrlOf(doc(""), home), null);
  assert.equal(syncUrlOf(null, home), null);
  const published = keyFromSeed(Buffer.alloc(32, 4)).published;
  let fetched = 0;
  const homeKeys = createHomeKeys({
    ownHome: "https://commy.chat",
    ownKey: { id: "own" },
    ownSync: "wss://commy.chat/sync",
    fetchDocument: async (asked) => {
      fetched += 1;
      return { issuer: asked, serverKey: published, endpoints: { sync: `wss://${new URL(asked).hostname}/sync` } };
    },
  });
  assert.deepEqual(await homeKeys.homeOf("https://commy.chat"), { key: { id: "own" }, sync: "wss://commy.chat/sync" });
  assert.deepEqual(await homeKeys.homeOf(home), { key: published, sync: "wss://friend.org/sync" });
  assert.deepEqual(await homeKeys.keyOf(home), published, "the key and the sync address share one look-up");
  assert.equal(fetched, 1);
});

test("a home that could not be reached is asked again within a minute", async () => {
  const home = "https://friend.org";
  const published = keyFromSeed(Buffer.alloc(32, 4)).published;
  let clock = 1_000_000;
  let reachable = false;
  let fetched = 0;
  const homeKeys = createHomeKeys({
    ownHome: "https://commy.chat",
    ownKey: { id: "own" },
    now: () => clock,
    fetchDocument: async (asked) => {
      fetched += 1;
      if (!reachable) throw new Error("timed out");
      return { issuer: asked, serverKey: published, endpoints: { sync: "wss://friend.org/sync" } };
    },
  });
  assert.deepEqual(await homeKeys.homeOf(home), { key: null, sync: null });
  reachable = true;
  clock += MISSING_FOR_MS - 1;
  assert.deepEqual(await homeKeys.homeOf(home), { key: null, sync: null }, "a miss is remembered briefly so a crowd cannot hammer the home");
  assert.equal(fetched, 1);
  assert.ok(MISSING_FOR_MS <= 60 * 1000);
  clock += 1;
  assert.deepEqual(await homeKeys.homeOf(home), { key: published, sync: "wss://friend.org/sync" });
  assert.equal(fetched, 2);
});

test("the list and server keys load from their own env names", () => {
  const config = { secret: SECRET };
  assert.equal(loadServerKey(config, { AUTH_SERVER_KEY: "05".repeat(32) }).published.public, keyFromSeed(Buffer.alloc(32, 5)).published.public);
  assert.equal(loadListKey(config, { COMMY_LIST_KEY: "06".repeat(32) }).published.public, keyFromSeed(Buffer.alloc(32, 6)).published.public);
});
