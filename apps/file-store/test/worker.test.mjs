import test from "node:test";
import assert from "node:assert/strict";
import { handle, SEALED_SIZES, MAX_FILE_PLAIN, objectName, usageName, fitsSealed, requestMessage, rosterMessage, verifies, forgetRoster, dailyCap, SIGNED_FOR_MS, DEFAULT_DAILY_UPLOAD_MB } from "../worker.js";

function memoryBucket() {
  const objects = new Map();
  return {
    objects,
    async head(name) {
      const body = objects.get(name);
      return body ? { size: body.byteLength } : null;
    },
    async get(name) {
      const body = objects.get(name);
      return body ? { size: body.byteLength, body: new Uint8Array(body), text: async () => new TextDecoder().decode(body) } : null;
    },
    async put(name, body) {
      const bytes = new Uint8Array(await new Response(body).arrayBuffer());
      objects.set(name, bytes);
      return { size: bytes.byteLength };
    },
    async delete(name) {
      objects.delete(name);
    },
  };
}

const hex = bytes => [...new Uint8Array(bytes)].map(b => b.toString(16).padStart(2, "0")).join("");

async function person() {
  const pair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
  const key = hex(await crypto.subtle.exportKey("raw", pair.publicKey));
  const sign = async text => hex(await crypto.subtle.sign({ name: "Ed25519" }, pair.privateKey, new TextEncoder().encode(text)));
  return { key, sign };
}

const NOW = Date.UTC(2026, 10, 5, 12);
const SETUP = "correct-horse-battery-staple";
const id = "ab".repeat(32);
const at = path => `https://commy-files.rockets.workers.dev${path}`;

function send(env, method, path, { body, headers = {}, now = NOW } = {}) {
  const sized = body ? { "content-length": String(body.byteLength ?? new TextEncoder().encode(body).byteLength), ...headers } : headers;
  return handle(new Request(at(path), { method, body, headers: sized }), env, now);
}

async function signedHeaders(who, method, target, length, when = NOW) {
  return { "x-commy-key": who.key, "x-commy-at": String(when), "x-commy-sig": await who.sign(requestMessage(method, target, when, length)) };
}

async function signedRoster(who, roster) {
  const text = JSON.stringify(roster);
  return JSON.stringify({ roster: text, key: who.key, sig: await who.sign(rosterMessage(text)) });
}

async function setUp() {
  forgetRoster();
  const [owner, manager, member, stranger] = await Promise.all([person(), person(), person(), person()]);
  const env = { FILES: memoryBucket(), SETUP_CODE: SETUP };
  const roster = { owner: owner.key, managers: [manager.key], members: [owner.key, manager.key, member.key], at: NOW };
  const res = await send(env, "POST", "/_commy/pair", { body: await signedRoster(owner, roster), headers: { "x-commy-setup": SETUP } });
  assert.equal(res.status, 204);
  return { env, owner, manager, member, stranger, roster };
}

async function put(env, who, bytes, now = NOW) {
  return send(env, "PUT", `/${id}`, { body: bytes, headers: await signedHeaders(who, "PUT", id, bytes.byteLength, now), now });
}

async function get(env, who, method = "GET") {
  return send(env, method, `/${id}`, { headers: await signedHeaders(who, method, id, 0) });
}

test("the root says this is Commy file storage, how big a file may be and whether it is set up", async () => {
  forgetRoster();
  const res = await send({ FILES: memoryBucket() }, "GET", "/");
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { commy_files: 2, max_bytes: MAX_FILE_PLAIN, paired: false });
  assert.equal(res.headers.get("access-control-allow-origin"), "*");
  const { env } = await setUp();
  assert.equal((await (await send(env, "GET", "/")).json()).paired, true);
});

test("signatures from the apps check out in the worker", async () => {
  const who = await person();
  const message = requestMessage("GET", id, NOW, 0);
  assert.ok(await verifies(who.key, message, await who.sign(message)));
  assert.ok(!(await verifies(who.key, requestMessage("PUT", id, NOW, 0), await who.sign(message))));
  assert.ok(!(await verifies("zz", message, "00")));
});

test("a request and a member list signed by the Commy apps check out", async () => {
  const signer = "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c";
  const sig = "ac92251a45380696c83571842764f431fb3987334cfe4078b6b56e5be7a2ab0399775d6cd2f8f69d7db0d92d3ce2d6768189f73c30975946de0b555bbc145702";
  assert.ok(await verifies(signer, requestMessage("GET", "ab".repeat(32), 1762430400000, 0), sig));
  const roster = `{"owner":"${signer}","managers":[],"members":["${signer}"],"at":1762430400000}`;
  const rosterSig = "8dcdd3d0c9dc197d99394d9cea0005708b1576c4a379a815cc4942955ad9c5573ff32db33c4bbaafcfb099bc27ed41761b55054ec031da1768deb656f2df190a";
  assert.ok(await verifies(signer, rosterMessage(roster), rosterSig));
});

test("pairing needs the setup code and a list signed by its owner or a manager", async () => {
  forgetRoster();
  const [owner, member] = await Promise.all([person(), person()]);
  const roster = { owner: owner.key, managers: [], members: [owner.key, member.key], at: NOW };
  const noCode = { FILES: memoryBucket() };
  assert.equal((await send(noCode, "POST", "/_commy/pair", { body: await signedRoster(owner, roster), headers: { "x-commy-setup": SETUP } })).status, 403, "a worker without a setup code never pairs");
  const shortCode = { FILES: memoryBucket(), SETUP_CODE: "short" };
  assert.equal((await send(shortCode, "POST", "/_commy/pair", { body: await signedRoster(owner, roster), headers: { "x-commy-setup": "short" } })).status, 403);
  const env = { FILES: memoryBucket(), SETUP_CODE: SETUP };
  assert.equal((await send(env, "POST", "/_commy/pair", { body: await signedRoster(owner, roster), headers: { "x-commy-setup": "wrong-code-wrong-code" } })).status, 403);
  assert.equal((await send(env, "POST", "/_commy/pair", { body: await signedRoster(member, roster), headers: { "x-commy-setup": SETUP } })).status, 400, "a plain member cannot pair");
  assert.equal((await send(env, "POST", "/_commy/pair", { body: await signedRoster(owner, roster), headers: { "x-commy-setup": ` ${SETUP} ` } })).status, 204);
});

test("only members on the list can read, check or upload files", async () => {
  const { env, owner, member, stranger } = await setUp();
  const sealed = new Uint8Array(SEALED_SIZES[0]).fill(7);
  assert.equal((await send(env, "PUT", `/${id}`, { body: sealed })).status, 403, "unsigned uploads are refused");
  assert.equal((await put(env, stranger, sealed)).status, 403);
  assert.equal((await put(env, member, sealed)).status, 204);
  assert.equal((await send(env, "GET", `/${id}`)).status, 403, "unsigned reads are refused");
  assert.equal((await get(env, stranger)).status, 403);
  assert.equal((await get(env, owner, "HEAD")).status, 200);
  const res = await get(env, member);
  assert.equal(res.status, 200);
  assert.equal(res.headers.get("cache-control"), "private, max-age=31536000, immutable");
  assert.deepEqual(new Uint8Array(await res.arrayBuffer()), sealed);
  const other = new Uint8Array(SEALED_SIZES[0]).fill(9);
  assert.equal((await put(env, owner, other)).status, 204);
  assert.deepEqual(env.FILES.objects.get(objectName(id)), sealed, "a kept file is never replaced");
});

test("a signature is good for one request, for five minutes", async () => {
  const { env, member } = await setUp();
  const stale = await signedHeaders(member, "GET", id, 0, NOW - SIGNED_FOR_MS - 1);
  assert.equal((await send(env, "GET", `/${id}`, { headers: stale })).status, 403);
  const forOther = await signedHeaders(member, "GET", "cd".repeat(32), 0);
  assert.equal((await send(env, "GET", `/${id}`, { headers: forOther })).status, 403);
  const headSig = await signedHeaders(member, "HEAD", id, 0);
  assert.equal((await send(env, "GET", `/${id}`, { headers: headSig })).status, 403);
  const sealed = new Uint8Array(SEALED_SIZES[0]);
  const small = await signedHeaders(member, "PUT", id, 10);
  assert.equal((await send(env, "PUT", `/${id}`, { body: sealed, headers: small })).status, 403, "the size is part of an upload's signature");
});

test("storage is closed until it is set up", async () => {
  forgetRoster();
  const who = await person();
  const env = { FILES: memoryBucket(), SETUP_CODE: SETUP };
  assert.equal((await put(env, who, new Uint8Array(SEALED_SIZES[0]))).status, 403);
  assert.equal(env.FILES.objects.size, 0);
});

test("the owner changes everything, managers change only members", async () => {
  const { env, owner, manager, member, stranger, roster } = await setUp();
  const withStranger = { ...roster, members: [...roster.members, stranger.key], at: NOW + 1 };
  assert.equal((await send(env, "PUT", "/_commy/roster", { body: await signedRoster(member, withStranger) })).status, 403);
  assert.equal((await send(env, "PUT", "/_commy/roster", { body: await signedRoster(manager, withStranger) })).status, 204);
  assert.equal((await put(env, stranger, new Uint8Array(SEALED_SIZES[0]))).status, 204);
  const promoted = { ...withStranger, managers: [manager.key, stranger.key], at: NOW + 2 };
  assert.equal((await send(env, "PUT", "/_commy/roster", { body: await signedRoster(manager, promoted) })).status, 403, "a manager cannot add managers");
  assert.equal((await send(env, "PUT", "/_commy/roster", { body: await signedRoster(owner, promoted) })).status, 204);
  const stolen = { ...promoted, owner: stranger.key, at: NOW + 3 };
  assert.equal((await send(env, "PUT", "/_commy/roster", { body: await signedRoster(stranger, stolen) })).status, 403, "the owner changes only with the setup code");
  assert.equal((await send(env, "PUT", "/_commy/roster", { body: await signedRoster(owner, { ...promoted, at: NOW + 2 }) })).status, 409, "an older or replayed list is refused");
  const removed = { ...promoted, managers: [manager.key], members: [owner.key, manager.key], at: NOW + 4 };
  assert.equal((await send(env, "PUT", "/_commy/roster", { body: await signedRoster(owner, removed) })).status, 204);
  assert.equal((await get(env, member)).status, 403, "someone who left can no longer read");
  assert.equal((await get(env, stranger)).status, 403);
});

test("owners and managers can read the list back, members cannot", async () => {
  const { env, owner, manager, member, roster } = await setUp();
  const res = await send(env, "GET", "/_commy/roster", { headers: await signedHeaders(manager, "GET", "roster", 0) });
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), roster);
  assert.equal((await send(env, "GET", "/_commy/roster", { headers: await signedHeaders(owner, "GET", "roster", 0) })).status, 200);
  assert.equal((await send(env, "GET", "/_commy/roster", { headers: await signedHeaders(member, "GET", "roster", 0) })).status, 403);
  assert.equal((await send(env, "GET", "/_commy/roster")).status, 403);
});

test("each member uploads at most the daily limit", async () => {
  const { env, member } = await setUp();
  env.DAILY_UPLOAD_MB = "1";
  assert.equal(dailyCap(env), 1024 * 1024);
  assert.equal(dailyCap({}), DEFAULT_DAILY_UPLOAD_MB * 1024 * 1024);
  const sealed = new Uint8Array(SEALED_SIZES[1]);
  const ids = ["01", "02", "03", "04", "05"].map(n => n.repeat(32));
  const upload = async (fileId, now = NOW) =>
    (await send(env, "PUT", `/${fileId}`, { body: sealed, headers: await signedHeaders(member, "PUT", fileId, sealed.byteLength, now), now })).status;
  assert.equal(await upload(ids[0]), 204);
  assert.equal(await upload(ids[1]), 204);
  assert.equal(await upload(ids[2]), 204);
  assert.equal(await upload(ids[3]), 429);
  assert.ok(!env.FILES.objects.has(objectName(ids[3])));
  assert.ok(env.FILES.objects.has(usageName(member.key)));
  const tomorrow = NOW + 24 * 60 * 60 * 1000;
  assert.equal(await upload(ids[4], tomorrow), 204, "the limit starts over each day");
});

test("only whole sealed sizes and real ids are taken", async () => {
  const { env, member } = await setUp();
  const odd = new Uint8Array(1000);
  assert.equal((await send(env, "PUT", `/${id}`, { body: odd, headers: await signedHeaders(member, "PUT", id, 1000) })).status, 400);
  assert.equal((await send(env, "PUT", "/not-an-id", { body: new Uint8Array(SEALED_SIZES[0]) })).status, 404);
  assert.equal((await send(env, "GET", `/${"AB".repeat(32)}`)).status, 404);
  assert.equal((await send(env, "DELETE", `/${id}`)).status, 405);
  assert.equal((await send(env, "POST", "/")).status, 405);
  assert.equal((await send(env, "GET", "/_commy/pair")).status, 405);
  const sealed = new Uint8Array(SEALED_SIZES[1]);
  const claimed = await signedHeaders(member, "PUT", id, SEALED_SIZES[0]);
  assert.equal((await send(env, "PUT", `/${id}`, { body: sealed, headers: { ...claimed, "content-length": String(SEALED_SIZES[0]) } })).status, 400, "a body that is not the size it claims is dropped");
  assert.ok(!env.FILES.objects.has(objectName(id)));
});

test("browsers may ask first, with the signature headers", async () => {
  const res = await send({ FILES: memoryBucket() }, "OPTIONS", `/${id}`);
  assert.equal(res.status, 204);
  assert.match(res.headers.get("access-control-allow-methods"), /PUT/);
  assert.match(res.headers.get("access-control-allow-headers"), /x-commy-sig/);
  assert.match(res.headers.get("access-control-allow-headers"), /x-commy-setup/);
});

test("a worker without its bucket says so", async () => {
  assert.equal((await send({}, "GET", "/")).status, 503);
});

test("sealed sizes match the app's buckets", () => {
  assert.equal(SEALED_SIZES.length, 10);
  assert.ok(fitsSealed(64 * 1024 + 16));
  assert.ok(fitsSealed(30 * 1024 * 1024 + 4 + 16));
  assert.ok(fitsSealed(24 * 1024 * 1024 + 16));
  assert.ok(!fitsSealed(64 * 1024));
});
