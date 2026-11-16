import assert from "node:assert/strict";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { isSealed, sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";
import { atLeast, checkServer, cleanOrigin, contactKind, createChecker, oldestFor, oldestSupported } from "../src/listing-checks.js";
import { createListingStore, openReviewToken, reviewToken, STATUS } from "../src/listing.js";
import { isDue, nextState, recheckDue } from "../src/listing-rechecks.js";
import { createServerList, entriesFrom, LIST_PURPOSE, SERVER_LIST_PATH } from "../src/server-list.js";
import { keyId, signedBytes } from "../src/server-key.js";
import { createPublicKey, verify } from "node:crypto";

const ORIGIN = "http://localhost:3001";
const SECRET = "test-secret-".repeat(4);
const ENV = { BETTER_AUTH_SECRET: SECRET, DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN, LISTING: "on", LISTING_REVIEW_EMAIL: "review@commy.test" };
const HOME = "https://chat.example.org";
const DAY = 24 * 60 * 60 * 1000;
const NOW = Date.now();
const SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
const PUBLIC = "11".repeat(32);
const GOOD_KEY = { alg: "ed25519", id: keyId(PUBLIC), public: PUBLIC };

function homeDocument(overrides = {}) {
  return {
    name: "Example Chat",
    issuer: HOME,
    serverKey: GOOD_KEY,
    release: "0.3.1",
    abuseContact: "abuse@example.org",
    legal: { terms: `${HOME}/terms`, privacy: `${HOME}/privacy` },
    features: { adultsOnly: true },
    ...overrides,
  };
}

function probes({ home = homeDocument(), sync = { protocol: 3, version: "0.3.1", operator: "Example", abuse_contact: "abuse@example.org" }, cert = { valid: true, validTo: NOW + 60 * DAY }, pages = {}, mail = true } = {}) {
  return {
    fetchJson: async (url) => {
      if (url.endsWith("/.well-known/commy-auth.json") && home) return home;
      if (url.endsWith("/sync/about") && sync) return sync;
      throw new Error("answered 404");
    },
    fetchPage: async (url) => pages[url] ?? { status: 200, type: "text/html; charset=utf-8" },
    certificate: async () => cert,
    acceptsMail: async () => mail,
  };
}

const run = (application, options = {}) => checkServer(application, { probes: probes(options), oldest: "0.2.0", now: NOW, ...options.extra });
const failed = (result) => result.checks.filter((c) => !c.ok).map((c) => c.id);

test("versions compare by number and the oldest supported is one minor back", () => {
  assert.ok(atLeast("0.10.0", "0.9.5"));
  assert.ok(atLeast("v0.2.0-rc.1", "0.2.0"));
  assert.ok(!atLeast("0.1.19", "0.2.0"));
  assert.ok(!atLeast("main", "0.2.0"));
  assert.ok(atLeast("0.0.1", null));
  assert.equal(oldestSupported("0.3.4"), "0.2.0");
  assert.equal(oldestSupported("1.0.2"), "1.0.0");
  assert.equal(oldestSupported("dev"), null);
  assert.equal(oldestFor(loadConfig({ ...ENV, COMMY_VERSION: "v0.4.0" })), "0.3.0");
  assert.equal(oldestFor(loadConfig({ ...ENV, COMMY_VERSION: "v0.4.0", LISTING_OLDEST_VERSION: "0.4.0" })), "0.4.0");
});

test("addresses and contacts are cleaned before anything is fetched", () => {
  assert.equal(cleanOrigin("Chat.Example.org/"), HOME);
  assert.equal(cleanOrigin("http://chat.example.org"), null);
  assert.equal(cleanOrigin("https://10.0.0.1"), null);
  assert.equal(cleanOrigin("https://chat.example.test"), null);
  assert.equal(contactKind("abuse@example.org").domain, "example.org");
  assert.equal(contactKind("https://example.org/abuse").kind, "page");
  assert.equal(contactKind("call me"), null);
});

test("a full server passing every check is a home and a sync host", async () => {
  const result = await run({ origin: "chat.example.org" });
  assert.deepEqual(failed(result), []);
  assert.ok(result.passed);
  assert.deepEqual(result.kinds, ["home", "sync"]);
  assert.equal(result.name, "Example Chat");
  assert.deepEqual(result.key, GOOD_KEY);
  assert.equal(result.terms, `${HOME}/terms`);
});

test("each broken promise fails its own check", async () => {
  assert.deepEqual(failed(await run({ origin: HOME }, { cert: { valid: false, problem: "CERT_HAS_EXPIRED" } })), ["tls"]);
  assert.deepEqual(failed(await run({ origin: HOME }, { cert: { valid: true, validTo: NOW + 5 * DAY } })), ["tls"]);
  assert.deepEqual(failed(await run({ origin: HOME }, { home: homeDocument({ release: "0.1.19" }), sync: null })), ["version"]);
  assert.deepEqual(failed(await run({ origin: HOME }, { home: homeDocument({ release: null }), sync: null })), ["version"]);
  assert.deepEqual(failed(await run({ origin: HOME }, { mail: false })), ["contact"]);
  assert.deepEqual(failed(await run({ origin: HOME }, { home: homeDocument({ abuseContact: null }), sync: null })), ["contact"]);
  assert.deepEqual(failed(await run({ origin: HOME }, { home: homeDocument({ legal: {} }) })), ["legal"]);
  assert.deepEqual(failed(await run({ origin: HOME }, { pages: { [`${HOME}/privacy`]: { status: 404, type: "text/html" } } })), ["legal"]);
  assert.deepEqual(failed(await run({ origin: HOME }, { home: homeDocument({ features: { adultsOnly: false } }) })), ["adults"]);
  assert.deepEqual(failed(await run({ origin: HOME }, { home: homeDocument({ issuer: "https://other.example.org" }) })), ["discovery"]);
  assert.deepEqual(failed(await run({ origin: HOME }, { home: null, sync: null })), ["discovery", "version", "contact", "legal"]);
});

test("a sync host on its own needs Terms and privacy links in the form and skips the 18+ check", async () => {
  const bare = await run({ origin: HOME }, { home: null });
  assert.deepEqual(failed(bare), ["legal"]);
  const linked = await run({ origin: HOME, terms: "/terms", privacy: `${HOME}/privacy` }, { home: null });
  assert.ok(linked.passed);
  assert.deepEqual(linked.kinds, ["sync"]);
  assert.equal(linked.terms, `${HOME}/terms`);
  const page = await run({ origin: HOME, terms: "/t", privacy: "/p" }, { home: null, sync: { protocol: 3, version: "0.3.0", abuse_contact: "https://example.org/abuse" } });
  assert.ok(page.passed);
  assert.equal(page.contactKind, "page");
});

test("a blocked or private address is refused before any request", async () => {
  let asked = false;
  const watching = { ...probes(), certificate: async () => ((asked = true), { valid: true }) };
  const blocked = await checkServer({ origin: HOME }, { probes: watching, blocked: (d) => d.endsWith("example.org"), now: NOW });
  assert.deepEqual(failed(blocked), ["address"]);
  assert.equal((await checkServer({ origin: "https://localhost" }, { probes: watching })).origin, null);
  assert.equal(asked, false);
});

test("review links are signed, bound to one application, and expire", () => {
  const row = { origin: HOME, appliedAt: new Date(NOW) };
  const token = reviewToken(SECRET, row, NOW);
  assert.deepEqual(openReviewToken(SECRET, token, NOW + DAY), { origin: HOME, appliedAt: NOW });
  assert.equal(openReviewToken(SECRET, token, NOW + 31 * DAY), null);
  assert.equal(openReviewToken("another-secret-".repeat(3), token, NOW), null);
  assert.equal(openReviewToken(SECRET, `${token.split(".")[0]}x.${token.split(".")[1]}`, NOW), null);
  assert.equal(openReviewToken(SECRET, "nonsense", NOW), null);
});

test("monthly re-checks give a failing server seven days, then delist it", () => {
  const listed = { status: STATUS.listed, checkedAt: new Date(NOW - 31 * DAY) };
  assert.ok(isDue(listed, NOW));
  assert.ok(!isDue({ ...listed, checkedAt: new Date(NOW - 29 * DAY) }, NOW));
  assert.ok(isDue({ status: STATUS.failing, checkedAt: new Date(NOW - DAY) }, NOW));
  assert.ok(!isDue({ status: STATUS.review, checkedAt: new Date(0) }, NOW));

  const broken = { passed: false, checks: [{ id: "tls", ok: false, detail: "The certificate is not accepted." }] };
  const fine = { passed: true, checks: [{ id: "tls", ok: true, detail: "ok" }] };
  const first = nextState(listed, broken, NOW);
  assert.equal(first.update.status, STATUS.failing);
  assert.equal(first.notice, "failing");
  const failing = { status: STATUS.failing, failingSince: new Date(NOW), checkedAt: new Date(NOW) };
  assert.equal(nextState(failing, broken, NOW + 3 * DAY).notice, null);
  assert.equal(nextState(failing, broken, NOW + 7 * DAY).update.status, STATUS.delisted);
  assert.deepEqual([nextState(failing, fine, NOW + 2 * DAY).update.status, nextState(failing, fine, NOW + 2 * DAY).notice], [STATUS.listed, "fixed"]);
  const blocked = { passed: false, checks: [{ id: "address", ok: false, detail: "on the list of blocked servers" }] };
  assert.equal(nextState(listed, blocked, NOW).update.status, STATUS.delisted);
});

test("the served list is signed with the list key and leaves out blocked servers", async () => {
  const config = loadConfig(ENV);
  const rows = [
    { origin: HOME, name: "Example", kinds: "home,sync", serverKey: JSON.stringify(GOOD_KEY), listedAt: new Date(NOW), checkedAt: new Date(NOW) },
    { origin: "https://bad.example.net", name: "Bad", kinds: "sync", listedAt: new Date(NOW), checkedAt: new Date(NOW) },
  ];
  assert.deepEqual(entriesFrom(rows, (d) => d === "bad.example.net").map((e) => e.origin), [HOME]);
  const served = createServerList({ listKey: config.listKey, origin: ORIGIN, loadRows: async () => rows, blocked: (d) => d === "bad.example.net", now: () => NOW });
  await served.refresh();
  let body = "";
  const res = { writeHead: (status) => assert.equal(status, 200), end: (text) => (body = text) };
  served.respond({ headers: {}, method: "GET" }, res);
  const envelope = JSON.parse(body);
  const list = JSON.parse(envelope.list);
  assert.equal(envelope.key, config.listKey.published.id);
  const key = createPublicKey({ key: Buffer.concat([SPKI_PREFIX, Buffer.from(config.listKey.published.public, "hex")]), format: "der", type: "spki" });
  assert.ok(verify(null, signedBytes(LIST_PURPOSE, ORIGIN, envelope.list), key, Buffer.from(envelope.sig, "hex")));
  assert.deepEqual(list.servers.map((s) => [s.origin, s.kinds, s.key?.id]), [[HOME, ["home", "sync"], GOOD_KEY.id]]);
  assert.equal(list.expires - list.issued, 7 * DAY);
});

test("discovery points at the list only where the listing is on, and names legal links and contact", () => {
  const on = discoveryDocument(loadConfig({ ...ENV, COMMY_TERMS_URL: "/terms", COMMY_ABUSE_CONTACT: "abuse@commy.test", COMMY_VERSION: "v0.3.0" }));
  assert.equal(on.serverList.url, `${ORIGIN}/api/auth${SERVER_LIST_PATH}`);
  assert.equal(on.legal.terms, `${ORIGIN}/terms`);
  assert.equal(on.abuseContact, "abuse@commy.test");
  assert.equal(on.release, "0.3.0");
  const off = discoveryDocument(loadConfig({ ...ENV, LISTING: "off" }));
  assert.equal(off.serverList, undefined);
  assert.throws(() => loadConfig({ ...ENV, COMMY_ABUSE_CONTACT: "nope" }), /COMMY_ABUSE_CONTACT/);
});

function listingServer(checkerProbes = probes()) {
  const config = loadConfig(ENV);
  const sent = [];
  const changes = [];
  const send = async (mail) => sent.push(mail);
  const checker = createChecker({ oldest: "0.2.0", probes: checkerProbes, now: () => NOW });
  const options = buildAuthOptions(config, { pool: null, send, listing: { checker, onChange: () => changes.push(1) } });
  const db = { user: [], session: [], account: [], verification: [], listedServer: [] };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  const auth = betterAuth(options);
  const call = async (method, path, body) => {
    const url = new URL(`${ORIGIN}/api/auth${path}`);
    if (method === "GET") for (const [k, v] of Object.entries(body ?? {})) url.searchParams.set(k, v);
    const init = { method, headers: { "content-type": "application/json" }, ...(method === "POST" ? { body: JSON.stringify(body) } : {}) };
    const res = await auth.handler(new Request(url, init));
    const text = await res.text();
    return { status: res.status, json: text ? JSON.parse(text) : null };
  };
  return { auth, call, sent, changes, db };
}

const APPLICATION = { origin: "chat.example.org", name: "Example Chat", about: "Friendly <b>folks</b>", operatorEmail: "op@example.org", emergencyEmail: "backup@example.org", pledge: true };

test("an application is checked, confirmed by its contact, reviewed, and then served", async () => {
  const { auth, call, sent, changes, db } = listingServer();
  const applied = await call("POST", "/commy/listing/apply", APPLICATION);
  assert.equal(applied.status, 200, JSON.stringify(applied.json));
  assert.equal(applied.json.passed, true);
  assert.equal(applied.json.confirmTo, "a***@example.org");
  assert.equal(sent[0].to, "abuse@example.org");
  assert.ok(isSealed(db.listedServer[0].operatorEmail) && isSealed(db.listedServer[0].emergencyEmail));
  assert.equal(db.listedServer[0].about, "Friendly bfolks/b");

  const again = await call("POST", "/commy/listing/apply", APPLICATION);
  assert.equal(again.json.passed, true, "a server still confirming can apply again for a fresh link");
  const token = new URL(sent[1].url).searchParams.get("confirm");
  assert.equal((await call("POST", "/commy/listing/confirm", { token: new URL(sent[0].url).searchParams.get("confirm") })).status, 404, "the older link no longer works");
  const confirmed = await call("POST", "/commy/listing/confirm", { token });
  assert.equal(confirmed.status, 200, JSON.stringify(confirmed.json));
  assert.equal(sent[2].to, "review@commy.test");
  assert.equal((await call("POST", "/commy/listing/confirm", { token })).status, 404, "a link works once");
  assert.equal((await call("POST", "/commy/listing/apply", APPLICATION)).status, 409);

  const review = new URL(sent[2].url).searchParams.get("token");
  const shown = await call("GET", "/commy/listing/review", { token: review });
  assert.equal(shown.json.application.operatorEmail, "op@example.org");
  assert.equal(shown.json.application.checks.length, 7);
  const decided = await call("POST", "/commy/listing/decide", { token: review, decision: "approve" });
  assert.equal(decided.json.status, STATUS.listed);
  assert.equal(sent[3].to, "op@example.org");
  assert.equal(changes.length, 1);
  assert.equal((await call("POST", "/commy/listing/decide", { token: review, decision: "reject" })).status, 404);

  const { adapter } = await auth.$context;
  const shownRows = await createListingStore(adapter).shown();
  assert.deepEqual(entriesFrom(shownRows).map((e) => [e.origin, e.name, e.kinds]), [[HOME, "Example Chat", ["home", "sync"]]]);
});

test("a failing application is not stored and says what to fix", async () => {
  const { call, sent, db } = listingServer(probes({ home: homeDocument({ features: { adultsOnly: false } }) }));
  const applied = await call("POST", "/commy/listing/apply", APPLICATION);
  assert.equal(applied.json.passed, false);
  assert.deepEqual(applied.json.checks.filter((c) => !c.ok).map((c) => c.id), ["adults"]);
  assert.equal(db.listedServer.length, 0);
  assert.equal(sent.length, 0);
  assert.equal((await call("POST", "/commy/listing/apply", { ...APPLICATION, pledge: false })).status, 400);
});

test("the monthly sweep re-checks due servers and emails the operator when one fails", async () => {
  let working = true;
  const live = probes();
  const flaky = { ...live, certificate: async () => (working ? { valid: true, validTo: NOW + 90 * DAY } : { valid: false, problem: "expired" }) };
  const { auth, call, sent } = listingServer(flaky);
  await call("POST", "/commy/listing/apply", APPLICATION);
  await call("POST", "/commy/listing/confirm", { token: new URL(sent[0].url).searchParams.get("confirm") });
  await call("POST", "/commy/listing/decide", { token: new URL(sent[1].url).searchParams.get("token"), decision: "approve" });
  const { adapter } = await auth.$context;
  const checker = createChecker({ oldest: "0.2.0", probes: flaky, now: () => NOW });
  const mails = [];
  const options = { checker, send: async (m) => mails.push(m), instance: "Commy", now: NOW + DAY };
  assert.deepEqual(await recheckDue(adapter, options), [], "nothing is due a day after listing");
  working = false;
  const [outcome] = await recheckDue(adapter, { ...options, now: NOW + 31 * DAY });
  assert.deepEqual([outcome.status, outcome.notice], [STATUS.failing, "failing"]);
  assert.deepEqual(mails.map((m) => m.to).sort(), ["abuse@example.org", "backup@example.org", "op@example.org"]);
  assert.match(mails[0].body, /certificate/);
  const [later] = await recheckDue(adapter, { ...options, now: NOW + 39 * DAY });
  assert.equal(later.status, STATUS.delisted);
  assert.deepEqual(await createListingStore(adapter).shown(), []);
});
