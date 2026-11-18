import assert from "node:assert/strict";
import { createPrivateKey, createPublicKey, sign } from "node:crypto";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { loadConfig } from "../src/config.js";
import { requestMessage } from "../src/home-card.js";
import { cleanCommyUser, cleanReason, REASON_MAX, runOperatorCommand, SUSPENDED_CODE, suspendedMessage } from "../src/suspension.js";

const ORIGIN = "http://localhost:3001";
const PASSWORD = "correct horse battery";
const PKCS8_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const TICKET = "sealed1." + "AbCd-_0123456789".repeat(5);
const KEY = "k".repeat(43);
const COMMY_USER = "6f1c1d6e-2b7a-4c55-9d4e-0d8a1b2c3d4e";
const OTHER_COMMY_USER = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
const ENV = {
  BETTER_AUTH_SECRET: "test-secret-".repeat(4),
  DATABASE_URL: "postgres://unused",
  BETTER_AUTH_URL: ORIGIN,
  COMMY_ABUSE_CONTACT: "safety@example.org",
};

function identity(fill) {
  const privateKey = createPrivateKey({ key: Buffer.concat([PKCS8_PREFIX, Buffer.alloc(32, fill)]), format: "der", type: "pkcs8" });
  const hex = createPublicKey(privateKey).export({ format: "der", type: "spki" }).subarray(12).toString("hex");
  return { hex, sign: (message) => sign(null, Buffer.from(message), privateKey).toString("hex") };
}

function start() {
  const options = buildAuthOptions(loadConfig(ENV), { pool: null, send: async () => {} });
  const db = {
    user: [], session: [], commyIdentity: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [],
    invite: [], inviteBackground: [], doorbellRing: [], directoryListing: [], commyMove: [],
  };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  const auth = betterAuth(options);
  const call = async (method, path, { body, cookie, query } = {}) => {
    const url = new URL(`${ORIGIN}/api/auth${path}`);
    for (const [k, v] of Object.entries(query ?? {})) url.searchParams.set(k, v);
    const headers = { origin: ORIGIN, cookie: cookie ?? "x=1" };
    if (body) headers["content-type"] = "application/json";
    const res = await auth.handler(new Request(url, { method, headers, body: body ? JSON.stringify(body) : undefined }));
    const text = await res.text();
    return { status: res.status, json: text ? JSON.parse(text) : null, setCookie: res.headers.get("set-cookie") };
  };
  const signUp = async (email) => {
    const r = await call("POST", "/sign-up/email", { body: { email, password: PASSWORD, name: "T" } });
    assert.equal(r.status, 200, JSON.stringify(r.json));
    return { cookie: r.setCookie.split(";")[0], id: r.json.user.id };
  };
  const signIn = (email) => call("POST", "/sign-in/email", { body: { email, password: PASSWORD } });
  const takeCard = (cookie, commyUser, person) => {
    const at = Date.now();
    const body = { user: commyUser, identity: person.hex, at, proof: person.sign(requestMessage(ORIGIN, commyUser, person.hex, at)) };
    return call("POST", "/commy/home-card", { body, cookie });
  };
  const operator = async (...words) => runOperatorCommand(await auth.$context, words);
  return { db, call, signUp, signIn, takeCard, operator };
}

async function listedServer(world, cookie, suffix = "") {
  const invite = await world.call("POST", "/commy/invites/create", { cookie, body: { ticket: TICKET + suffix, expiresIn: "never" } });
  assert.equal(invite.status, 200, JSON.stringify(invite.json));
  const code = invite.json.code;
  const listed = await world.call("POST", "/commy/directory/publish", {
    cookie,
    body: { code, key: KEY, name: `Loud Place${suffix}`, description: "", category: "friends", members: 25 },
  });
  assert.equal(listed.status, 200, JSON.stringify(listed.json));
  return code;
}

const discoverCodes = async (world) => (await world.call("GET", "/commy/directory/list")).json.listings.map((row) => row.code);

test("Commy ids and reasons are checked before anything is looked up", () => {
  assert.equal(cleanCommyUser(` ${COMMY_USER.toUpperCase()} `), COMMY_USER);
  assert.equal(cleanCommyUser("someone@example.org"), null);
  assert.equal(cleanCommyUser(""), null);
  assert.equal(cleanReason("  spam \n links "), "spam links");
  assert.equal(cleanReason(""), null);
  assert.equal(cleanReason("x".repeat(REASON_MAX + 1)), null);
  assert.match(suspendedMessage("safety@example.org"), /write to safety@example\.org/);
  assert.match(suspendedMessage(null), /this account server's contact/);
});

test("a home card links the account to its Commy id once", async () => {
  const world = start();
  const person = identity(3);
  const { cookie, id } = await world.signUp("ada@example.org");
  assert.equal((await world.takeCard(cookie, COMMY_USER, person)).status, 200);
  assert.equal((await world.takeCard(cookie, COMMY_USER, person)).status, 200);
  assert.deepEqual(world.db.commyIdentity.map((row) => [row.commyUser, row.userId]), [[COMMY_USER, id]]);
  assert.match(await world.operator("find", COMMY_USER), /ada@example\.org {2}active/);
  assert.match(await world.operator("find", "ADA@example.org"), new RegExp(`Commy ids: ${COMMY_USER}`));
});

test("suspending signs the account out everywhere and refuses new sign-ins with a clear code", async () => {
  const world = start();
  const { cookie } = await world.signUp("bea@example.org");
  await world.takeCard(cookie, COMMY_USER, identity(4));
  const second = await world.signIn("bea@example.org");
  assert.equal(second.status, 200);
  assert.equal(world.db.session.length, 2);

  const said = await world.operator("suspend", COMMY_USER, "harassment", "in", "DMs");
  assert.match(said, /Suspended 1 account\./);
  assert.match(said, /SUSPENDED .* \(harassment in DMs\)/);
  assert.equal(world.db.session.length, 0);
  assert.equal((await world.call("GET", "/get-session", { cookie })).json, null);

  const refused = await world.signIn("bea@example.org");
  assert.equal(refused.status, 403);
  assert.equal(refused.json.code, SUSPENDED_CODE);
  assert.match(refused.json.message, /safety@example\.org/);
  assert.equal(world.db.session.length, 0);
  assert.match(await world.operator("list"), /bea@example\.org {2}SUSPENDED/);

  assert.match(await world.operator("lift", "bea@example.org"), /Lifted the suspension on 1 account\./);
  assert.equal((await world.signIn("bea@example.org")).status, 200);
  assert.equal(await world.operator("list"), "No suspended accounts.\n");
});

test("the suspension stays out of what the account server returns", async () => {
  const world = start();
  const { cookie } = await world.signUp("cy@example.org");
  const session = (await world.call("GET", "/get-session", { cookie })).json;
  assert.ok(session.user);
  assert.equal("suspendedAt" in session.user, false);
  assert.equal("suspendedReason" in session.user, false);
});

test("a suspended account's Discover listings and invite links stop showing, and come back when lifted", async () => {
  const world = start();
  const bad = await world.signUp("dee@example.org");
  const good = await world.signUp("eve@example.org");
  const badCode = await listedServer(world, bad.cookie, "a");
  const goodCode = await listedServer(world, good.cookie, "b");
  assert.deepEqual((await discoverCodes(world)).sort(), [badCode, goodCode].sort());

  await world.operator("suspend", "dee@example.org", "spam");
  assert.deepEqual(await discoverCodes(world), [goodCode]);
  assert.equal((await world.call("GET", "/commy/invites/resolve", { query: { code: badCode } })).status, 404);
  assert.equal((await world.call("GET", "/commy/invites/resolve", { query: { code: goodCode } })).status, 200);
  assert.equal(world.db.directoryListing.length, 2, "listings are hidden, not deleted");

  await world.operator("lift", "dee@example.org");
  assert.deepEqual((await discoverCodes(world)).sort(), [badCode, goodCode].sort());
  assert.equal((await world.call("GET", "/commy/invites/resolve", { query: { code: badCode } })).status, 200);
});

test("every account that claimed a reported Commy id is suspended, and nobody else", async () => {
  const world = start();
  const owner = await world.signUp("fay@example.org");
  const copycat = await world.signUp("gus@example.org");
  const bystander = await world.signUp("hal@example.org");
  await world.takeCard(owner.cookie, COMMY_USER, identity(5));
  await world.takeCard(copycat.cookie, COMMY_USER, identity(6));
  await world.takeCard(bystander.cookie, OTHER_COMMY_USER, identity(7));

  assert.match(await world.operator("suspend", COMMY_USER, "threats"), /Suspended 2 accounts\./);
  const listed = await world.operator("list");
  assert.match(listed, /fay@example\.org/);
  assert.match(listed, /gus@example\.org/);
  assert.doesNotMatch(listed, /hal@example\.org/);
  assert.equal((await world.signIn("hal@example.org")).status, 200);
});

test("the operator commands say what they need", async () => {
  const world = start();
  await world.signUp("ivy@example.org");
  await assert.rejects(world.operator("suspend", "ivy@example.org"), /say why/);
  await assert.rejects(world.operator("suspend", "not-an-id", "spam"), /Usage/);
  await assert.rejects(world.operator("wipe", "ivy@example.org"), /Usage/);
  assert.match(await world.operator("find", OTHER_COMMY_USER), /No account here has used/);
  assert.match(await world.operator("suspend", "nobody@example.org", "spam"), /No account here has used/);
});
