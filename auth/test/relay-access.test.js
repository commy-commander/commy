import assert from "node:assert/strict";
import http from "node:http";
import { after, before, test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { loadConfig } from "../src/config.js";
import { createRequestHandler } from "../src/http.js";
import { createRelayAccess, linkedDeviceLookup, RELAY_ACCESS_PATH, relayAccessToken } from "../src/relay-access.js";

const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: "http://localhost:3001" };
const TOKEN = "relay-access-token-".repeat(3);
const LINKED = "ab".repeat(32);
const STRANGER = "cd".repeat(32);

let server;
let base;

before(async () => {
  const config = loadConfig(ENV);
  const options = buildAuthOptions(config, { pool: null, send: async () => {} });
  options.database = sealedAtRest(memoryAdapter({ user: [], session: [], account: [], verification: [], linkedDevice: [] }), options.secret);
  const auth = betterAuth(options);
  const { adapter } = await auth.$context;
  const user = await adapter.create({ model: "user", data: { name: "Linked", email: "linked@example.com", emailVerified: true, createdAt: new Date(), updatedAt: new Date() } });
  await adapter.create({ model: "linkedDevice", data: { nodeId: LINKED, userId: user.id, createdAt: new Date(), lastSeenAt: new Date() } });
  const relayAccess = createRelayAccess({ token: TOKEN, isLinked: linkedDeviceLookup(auth.$context) });
  const handler = createRequestHandler({
    authHandler: (_req, res) => res.end("auth"),
    basePath: config.basePath,
    discovery: "{}",
    pool: { query: async () => {} },
    redis: null,
    relayAccess,
  });
  server = http.createServer(handler);
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  base = `http://127.0.0.1:${server.address().port}`;
});

after(() => server.close());

function ask(endpoint, { token = TOKEN, method = "POST" } = {}) {
  const headers = { "x-iroh-endpoint-id": endpoint };
  if (token) headers.authorization = `Bearer ${token}`;
  return fetch(`${base}${RELAY_ACCESS_PATH}`, { method, headers });
}

test("a device linked to an account is let onto the relay", async () => {
  const res = await ask(LINKED);
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { linked: true });
});

test("an endpoint no account has linked is not", async () => {
  const res = await ask(STRANGER);
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { linked: false });
});

test("upper case endpoint ids are the same device", async () => {
  assert.deepEqual(await (await ask(LINKED.toUpperCase())).json(), { linked: true });
});

test("only the relay holding the token may ask", async () => {
  assert.equal((await ask(LINKED, { token: null })).status, 401);
  assert.equal((await ask(LINKED, { token: "wrong" })).status, 401);
  assert.equal((await ask(LINKED, { method: "GET" })).status, 405);
});

test("a malformed endpoint id is refused", async () => {
  assert.equal((await ask("not-hex")).status, 400);
});

test("the check stays off until a long enough token is set", () => {
  assert.equal(relayAccessToken({}), null);
  assert.equal(relayAccessToken({ RELAY_ACCESS_TOKEN: "short" }), null);
  assert.equal(relayAccessToken({ RELAY_ACCESS_TOKEN: ` ${TOKEN} ` }), TOKEN);
  assert.equal(createRelayAccess({ token: null, isLinked: async () => true }).enabled, false);
});

test("without the check the path is just not found", async () => {
  const config = loadConfig(ENV);
  const handler = createRequestHandler({
    authHandler: (_req, res) => res.end("auth"),
    basePath: config.basePath,
    discovery: "{}",
    pool: { query: async () => {} },
    redis: null,
    relayAccess: createRelayAccess({ token: null, isLinked: async () => true }),
  });
  const off = http.createServer(handler);
  await new Promise((resolve) => off.listen(0, "127.0.0.1", resolve));
  try {
    const res = await fetch(`http://127.0.0.1:${off.address().port}${RELAY_ACCESS_PATH}`, { method: "POST" });
    assert.equal(res.status, 404);
  } finally {
    off.close();
  }
});
