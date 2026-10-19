import assert from "node:assert/strict";
import { generateKeyPairSync, sign } from "node:crypto";
import { before, test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { buildAuthOptions } from "../src/auth.js";
import { loadConfig } from "../src/config.js";

const ORIGIN = "http://localhost:3001";
let auth;
let db;

function nodeKey() {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const nodeId = publicKey.export({ format: "der", type: "spki" }).subarray(12).toString("hex");
  return { nodeId, sign: (message) => sign(null, Buffer.from(message), privateKey).toString("hex") };
}

before(() => {
  const config = loadConfig({
    BETTER_AUTH_SECRET: "test-secret-".repeat(4),
    DATABASE_URL: "postgres://unused",
    BETTER_AUTH_URL: ORIGIN,
  });
  const options = buildAuthOptions(config, { pool: null, send: async () => {} });
  db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], doorbellRing: [], twoFactor: [], passkey: [] };
  options.database = memoryAdapter(db);
  options.rateLimit = { enabled: false };
  auth = betterAuth(options);
});

async function call(method, path, { body, cookie, token, query, ip } = {}) {
  const url = new URL(`${ORIGIN}/api/auth${path}`);
  for (const [k, v] of Object.entries(query ?? {})) url.searchParams.set(k, v);
  const headers = { origin: ORIGIN, "user-agent": "test-agent/1.0" };
  if (body) headers["content-type"] = "application/json";
  if (cookie) headers.cookie = cookie;
  if (token) headers.authorization = `Bearer ${token}`;
  if (ip) headers["x-forwarded-for"] = ip;
  const res = await auth.handler(new Request(url, { method, headers, body: body ? JSON.stringify(body) : undefined }));
  const text = await res.text();
  return { status: res.status, json: text ? JSON.parse(text) : null, setCookie: res.headers.get("set-cookie") };
}

async function signUp(email) {
  const r = await call("POST", "/sign-up/email", {
    body: { email, password: "correct horse battery", name: "Tester" },
    ip: "198.51.100.7",
  });
  assert.equal(r.status, 200, JSON.stringify(r.json));
  return r.setCookie.split(";")[0];
}

async function desktopSignIn(cookie, { appIp = "203.0.113.5", approverIp = "203.0.113.5" } = {}) {
  const code = await call("POST", "/device/code", { body: { client_id: "commy-desktop" }, ip: appIp });
  assert.equal(code.status, 200, JSON.stringify(code.json));
  assert.match(code.json.verification_uri, /\/device$/);

  const tokenReq = {
    body: {
      grant_type: "urn:ietf:params:oauth:grant-type:device_code",
      device_code: code.json.device_code,
      client_id: "commy-desktop",
    },
  };
  const pending = await call("POST", "/device/token", tokenReq);
  assert.equal(pending.json.error, "authorization_pending");

  const context = await call("GET", "/commy/device-request", {
    cookie,
    ip: approverIp,
    query: { user_code: code.json.user_code.toLowerCase() },
  });
  assert.equal(context.status, 200, JSON.stringify(context.json));
  assert.equal(context.json.found, true);
  assert.equal(context.json.sameNetwork, appIp === approverIp);

  const verify = await call("GET", "/device", { cookie, query: { user_code: code.json.user_code } });
  assert.equal(verify.json.status, "pending");
  const approve = await call("POST", "/device/approve", { cookie, body: { userCode: code.json.user_code } });
  assert.equal(approve.status, 200, JSON.stringify(approve.json));

  await new Promise((r) => setTimeout(r, 5100));
  const granted = await call("POST", "/device/token", tokenReq);
  assert.equal(granted.status, 200, JSON.stringify(granted.json));
  assert.equal(granted.json.token_type, "Bearer");
  return granted.json.access_token;
}

async function link(auth, key, name) {
  const ch = await call("POST", "/commy/devices/challenge", { ...auth, body: { nodeId: key.nodeId } });
  assert.equal(ch.status, 200, JSON.stringify(ch.json));
  assert.ok(ch.json.message.startsWith("commy-device-link:v1\nhttp://localhost:3001\n"));
  return call("POST", "/commy/devices/register", {
    ...auth,
    body: { nodeId: key.nodeId, name, challenge: ch.json.challenge, signature: key.sign(ch.json.message) },
  });
}

test("unknown desktop client ids are rejected", async () => {
  const r = await call("POST", "/device/code", { body: { client_id: "someone-else" } });
  assert.equal(r.status, 400);
});

test("device routes require a session", async () => {
  assert.equal((await call("GET", "/commy/devices")).status, 401);
  assert.equal((await call("POST", "/commy/devices/challenge", { body: { nodeId: "ab".repeat(32) } })).status, 401);
  assert.equal((await call("GET", "/commy/device-request", { query: { user_code: "X" } })).status, 401);
});

test("sessions never store IP addresses or user agents", async () => {
  await signUp("privacy@example.test");
  assert.ok(db.session.length > 0);
  for (const s of db.session) {
    assert.equal(s.ipAddress ?? null, null);
    assert.equal(s.userAgent ?? null, null);
  }
});

test("approval context flags a request from another network", { timeout: 30000 }, async () => {
  const cookie = await signUp("carol@example.test");
  await desktopSignIn(cookie, { appIp: "192.0.2.10", approverIp: "203.0.113.99" });
  assert.ok(db.deviceRequest.every((r) => !String(r.network).includes("192.0.2.10")), "raw IPs are never stored");
});

test("desktop sign in, prove key, link, list, transfer and remove", { timeout: 30000 }, async () => {
  const cookie = await signUp("alice@example.test");
  const token = await desktopSignIn(cookie);
  const key = nodeKey();

  const session = await call("GET", "/get-session", { token });
  assert.equal(session.json.user.email, "alice@example.test");

  const bad = await call("POST", "/commy/devices/challenge", { token, body: { nodeId: "nothex" } });
  assert.equal(bad.status, 400);

  const ch = await call("POST", "/commy/devices/challenge", { token, body: { nodeId: key.nodeId } });
  const forged = await call("POST", "/commy/devices/register", {
    token,
    body: { nodeId: key.nodeId, challenge: ch.json.challenge, signature: nodeKey().sign(ch.json.message) },
  });
  assert.equal(forged.status, 403);

  const bob = await signUp("bob@example.test");
  const stolen = await call("POST", "/commy/devices/register", {
    cookie: bob,
    body: { nodeId: key.nodeId, challenge: ch.json.challenge, signature: key.sign(ch.json.message) },
  });
  assert.equal(stolen.status, 403);

  const reg = await link({ token }, key, "Desk PC");
  assert.equal(reg.status, 200, JSON.stringify(reg.json));
  assert.equal(reg.json.device.nodeId, key.nodeId);

  const again = await link({ token }, key, "DESKTOP-ABC123");
  assert.equal(again.status, 200, JSON.stringify(again.json));
  assert.equal(again.json.device.name, "Commy desktop", "the name an app sends is not kept");
  for (const row of db.linkedDevice) {
    assert.deepEqual(Object.keys(row).sort(), ["createdAt", "id", "lastSeenAt", "nodeId", "sessionId", "userId"]);
  }

  const list = await call("GET", "/commy/devices", { cookie });
  assert.equal(list.json.devices.length, 1);
  assert.equal(list.json.devices[0].name, "Commy desktop");

  await call("POST", "/commy/devices/remove", { cookie: bob, body: { id: list.json.devices[0].id } });
  assert.equal((await call("GET", "/commy/devices", { token })).json.devices.length, 1, "bob cannot remove another user's device");

  const moved = await link({ cookie: bob }, key, "Now Bob's");
  assert.equal(moved.status, 200, JSON.stringify(moved.json));
  assert.equal((await call("GET", "/commy/devices", { token })).json.devices.length, 0);
  const bobs = await call("GET", "/commy/devices", { cookie: bob });
  assert.equal(bobs.json.devices.length, 1);

  await call("POST", "/commy/devices/remove", { cookie: bob, body: { id: bobs.json.devices[0].id } });
  assert.equal((await call("GET", "/commy/devices", { cookie: bob })).json.devices.length, 0);
});

test("removing a device signs that app out and keeps the website signed in", { timeout: 30000 }, async () => {
  const cookie = await signUp("dana@example.test");
  const token = await desktopSignIn(cookie);
  const reg = await link({ token }, nodeKey(), "Laptop");
  assert.equal(reg.status, 200, JSON.stringify(reg.json));
  assert.equal((await call("GET", "/get-session", { token })).json.user.email, "dana@example.test");

  const removed = await call("POST", "/commy/devices/remove", { cookie, body: { id: reg.json.device.id } });
  assert.equal(removed.status, 200, JSON.stringify(removed.json));
  assert.equal(removed.json.signedOut, true);
  assert.equal((await call("GET", "/get-session", { token })).json, null, "the app's token stops working");
  assert.equal((await call("GET", "/commy/devices", { token })).status, 401);
  assert.equal((await call("GET", "/get-session", { cookie })).json.user.email, "dana@example.test");
});

test("a device cannot sign out a session that belongs to someone else", { timeout: 30000 }, async () => {
  const erin = await signUp("erin@example.test");
  const frank = await signUp("frank@example.test");
  const erinToken = await desktopSignIn(erin);
  const key = nodeKey();
  await link({ token: erinToken }, key, "Shared");
  const moved = await link({ cookie: frank }, key, "Shared");
  assert.equal(moved.status, 200, JSON.stringify(moved.json));
  await call("POST", "/commy/devices/remove", { cookie: frank, body: { id: moved.json.device.id } });
  assert.equal((await call("GET", "/get-session", { cookie: frank })).json.user.email, "frank@example.test");
  assert.equal((await call("GET", "/get-session", { token: erinToken })).json.user.email, "erin@example.test");
});
