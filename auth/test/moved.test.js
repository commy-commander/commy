import assert from "node:assert/strict";
import { createPrivateKey, createPublicKey, sign } from "node:crypto";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";
import { checkMoveRequest, commyMoved, KEPT_FOR_MS, moveMessage, pointerOf, widenMoveTimes } from "../src/moved.js";

const ORIGIN = "http://localhost:3001";
const SECRET = "test-secret-".repeat(4);
const ENV = { BETTER_AUTH_SECRET: SECRET, DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN };
const PKCS8_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const USER = "6f1c1d6e-2b7a-4c55-9d4e-0d8a1b2c3d4e";
const NEW_HOME = "https://friends.org";

function identity(fill = 9) {
  const privateKey = createPrivateKey({ key: Buffer.concat([PKCS8_PREFIX, Buffer.alloc(32, fill)]), format: "der", type: "pkcs8" });
  const hex = createPublicKey(privateKey).export({ format: "der", type: "spki" }).subarray(12).toString("hex");
  return { hex, sign: (message) => sign(null, Buffer.from(message), privateKey).toString("hex") };
}

function moveBody(person, to, at = Date.now(), home = ORIGIN) {
  return { to, user: USER, identity: person.hex, at, proof: person.sign(moveMessage(home, to, USER, person.hex, at)) };
}

test("a move must be signed by the identity it moves and be recent", () => {
  const person = identity();
  const now = 1_800_000_000_000;
  assert.equal(checkMoveRequest(ORIGIN, moveBody(person, NEW_HOME, now), now), null);
  assert.match(checkMoveRequest(ORIGIN, { ...moveBody(person, NEW_HOME, now), proof: "00".repeat(64) }, now), /did not sign/);
  assert.match(checkMoveRequest(ORIGIN, moveBody(person, NEW_HOME, now - 11 * 60 * 1000), now), /too old/);
  assert.match(checkMoveRequest(ORIGIN, moveBody(person, "http://friends.org", now), now), /public domain/);
  assert.match(checkMoveRequest(ORIGIN, moveBody(person, "https://box.local", now), now), /public domain/);
  assert.match(checkMoveRequest(ORIGIN, { ...moveBody(person, NEW_HOME, now), user: "someone" }, now), /Commy user id/);
  assert.equal(checkMoveRequest(ORIGIN, moveBody(person, ORIGIN, now), now), null, "moving back home cancels the pointer");
  const elsewhere = moveBody(person, NEW_HOME, now, "https://other.org");
  assert.match(checkMoveRequest(ORIGIN, elsewhere, now), /did not sign/, "a move signed for another home is not ours to record");
});

test("an old pointer is no longer served", () => {
  const row = { identity: "ab", commyUser: USER, toHome: NEW_HOME, at: 5, proof: "p" };
  assert.deepEqual(pointerOf(ORIGIN, row, 6), { from: ORIGIN, to: NEW_HOME, user: USER, identity: "ab", at: 5, proof: "p" });
  assert.equal(pointerOf(ORIGIN, row, 6 + KEPT_FOR_MS), null);
  assert.equal(pointerOf(ORIGIN, null, 6), null);
});

test("a move time is kept in milliseconds, which needs a bigint column", () => {
  const at = commyMoved({ home: ORIGIN }).schema.commyMove.fields.at;
  assert.equal(at.type, "number");
  assert.equal(at.bigint, true);
  assert.ok(Date.now() > 2 ** 31, "today's millisecond time does not fit a Postgres integer");
});

function columnPool(dataType) {
  const sent = [];
  return {
    sent,
    query: async (text) => {
      sent.push(text);
      return { rows: text.startsWith("SELECT") && dataType ? [{ data_type: dataType }] : [] };
    },
  };
}

test("an older integer move time column is widened once, keeping its rows", async () => {
  const old = columnPool("integer");
  assert.equal(await widenMoveTimes(old), true);
  assert.equal(old.sent.length, 2);
  assert.match(old.sent[1], /^ALTER TABLE "commyMove" ALTER COLUMN "at" TYPE bigint$/);
  for (const ready of [columnPool("bigint"), columnPool(null)]) {
    assert.equal(await widenMoveTimes(ready), false);
    assert.equal(ready.sent.length, 1, "a bigint column or a fresh database is left alone");
  }
});

test("discovery tells apps where moves are recorded", () => {
  const doc = discoveryDocument(loadConfig(ENV));
  assert.equal(doc.endpoints.moved, `${ORIGIN}/api/auth/commy/moved`);
  assert.equal(doc.endpoints.movedSet, `${ORIGIN}/api/auth/commy/moved/set`);
  assert.equal(doc.features.moving, true);
});

test("a signed-in person records a move, anyone can read it, and moving back removes it", async () => {
  const config = loadConfig(ENV);
  const options = buildAuthOptions(config, { pool: null, send: async () => {}, fetch });
  const db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], commyMove: [] };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  const auth = betterAuth(options);
  const call = async (path, { body, cookie, method = "POST" } = {}) => {
    const headers = { "content-type": "application/json", ...(cookie ? { origin: ORIGIN, cookie } : {}) };
    const init = { method, headers, ...(method === "POST" ? { body: JSON.stringify(body) } : {}) };
    const res = await auth.handler(new Request(`${ORIGIN}/api/auth${path}`, init));
    const text = await res.text();
    return { status: res.status, json: text ? JSON.parse(text) : null, setCookie: res.headers.get("set-cookie") ?? "" };
  };
  const person = identity(7);
  const read = () => call(`/commy/moved?identity=${person.hex}`, { method: "GET" });
  assert.equal((await call("/commy/moved/set", { body: moveBody(person, NEW_HOME) })).status, 401);
  assert.equal((await read()).status, 404);
  const signUp = await call("/sign-up/email", { cookie: "x=1", body: { email: "mover@example.org", password: "correct horse battery", name: "M" } });
  const cookie = signUp.setCookie.split(";")[0];
  const at = Date.now();
  const recorded = await call("/commy/moved/set", { body: moveBody(person, NEW_HOME, at), cookie });
  assert.equal(recorded.status, 200, JSON.stringify(recorded.json));
  const seen = await read();
  assert.equal(seen.status, 200);
  assert.deepEqual(seen.json.moved, { from: ORIGIN, to: NEW_HOME, user: USER, identity: person.hex, at, proof: moveBody(person, NEW_HOME, at).proof });
  assert.equal(db.commyMove[0].userId, JSON.parse(JSON.stringify(signUp.json)).user.id, "the note belongs to the account that wrote it, so deleting the account removes it");
  const stale = await call("/commy/moved/set", { body: moveBody(person, "https://other.org", at - 1000), cookie });
  assert.equal(stale.status, 409, "an older move cannot replace a newer one");
  const back = await call("/commy/moved/set", { body: moveBody(person, ORIGIN, at + 1000), cookie });
  assert.equal(back.status, 200);
  assert.equal((await read()).status, 404);
});
