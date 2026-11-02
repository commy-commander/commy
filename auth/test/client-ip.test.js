import assert from "node:assert/strict";
import http from "node:http";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { toNodeHandler } from "better-auth/node";
import { buildAuthOptions } from "../src/auth.js";
import { clientIp, PRIVATE_NETWORKS, trustedProxyList } from "../src/client-ip.js";
import { loadConfig } from "../src/config.js";
import { createRequestHandler } from "../src/http.js";

const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: "http://localhost:3001" };
const edge = trustedProxyList(PRIVATE_NETWORKS);

test("a visitor named by the edge proxy is told apart by their own address", () => {
  assert.equal(clientIp({ peer: "172.20.0.4", forwardedFor: "198.51.100.7", trustedProxies: edge }), "198.51.100.7");
  assert.equal(clientIp({ peer: "::ffff:172.20.0.4", forwardedFor: "2001:db8::7", trustedProxies: edge }), "2001:db8::7");
});

test("the website passing a visitor along keeps the visitor's address", () => {
  assert.equal(clientIp({ peer: "172.20.0.9", forwardedFor: "198.51.100.7, 172.20.0.4", trustedProxies: edge }), "198.51.100.7");
});

test("a visitor reaching the server directly cannot claim another address", () => {
  assert.equal(clientIp({ peer: "203.0.113.5", forwardedFor: "198.51.100.7", trustedProxies: edge }), "203.0.113.5");
  assert.equal(clientIp({ peer: "::ffff:203.0.113.5", forwardedFor: undefined, trustedProxies: edge }), "203.0.113.5");
});

test("only the hop the trusted proxy added is believed", () => {
  assert.equal(clientIp({ peer: "172.20.0.4", forwardedFor: "6.6.6.6, 198.51.100.7", trustedProxies: edge }), "198.51.100.7");
  assert.equal(clientIp({ peer: "172.20.0.4", forwardedFor: "198.51.100.7, not-an-ip", trustedProxies: edge }), "172.20.0.4");
});

test("an app on the same machine or network without a proxy is named by its own address", () => {
  assert.equal(clientIp({ peer: "::ffff:127.0.0.1", forwardedFor: undefined, trustedProxies: edge }), "127.0.0.1");
  assert.equal(clientIp({ peer: "192.168.1.20", forwardedFor: "", trustedProxies: edge }), "192.168.1.20");
  assert.equal(clientIp({ peer: "172.20.0.4", forwardedFor: "10.0.0.8", trustedProxies: edge }), "10.0.0.8");
});

test("with no trusted proxies the connection's own address is used", () => {
  const none = trustedProxyList([]);
  assert.equal(clientIp({ peer: "172.20.0.4", forwardedFor: "198.51.100.7", trustedProxies: none }), "172.20.0.4");
});

test("TRUSTED_PROXIES picks which proxies may name the visitor", () => {
  const custom = loadConfig({ ...ENV, TRUSTED_PROXIES: "203.0.113.0/24, 2001:db8:1::/48" }).trustedProxies;
  assert.equal(clientIp({ peer: "203.0.113.9", forwardedFor: "198.51.100.7", trustedProxies: custom }), "198.51.100.7");
  assert.equal(clientIp({ peer: "172.20.0.4", forwardedFor: "198.51.100.7", trustedProxies: custom }), "172.20.0.4");
  const none = loadConfig({ ...ENV, TRUSTED_PROXIES: "none" }).trustedProxies;
  assert.equal(clientIp({ peer: "127.0.0.1", forwardedFor: "198.51.100.7", trustedProxies: none }), "127.0.0.1");
  assert.throws(() => loadConfig({ ...ENV, TRUSTED_PROXIES: "10.0.0.0/40" }), /TRUSTED_PROXIES/);
  assert.throws(() => loadConfig({ ...ENV, TRUSTED_PROXIES: "proxy.example" }), /TRUSTED_PROXIES/);
});

function serve(trustedProxies) {
  const config = loadConfig(ENV);
  const options = buildAuthOptions(config, { pool: null, send: async () => {} });
  options.database = memoryAdapter({ user: [], session: [], account: [], verification: [], deviceCode: [] });
  const handler = createRequestHandler({
    authHandler: toNodeHandler(betterAuth(options)),
    basePath: config.basePath,
    discovery: "{}",
    pool: { query: async () => {} },
    redis: null,
    trustedProxies,
  });
  const server = http.createServer(handler);
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(server)));
}

async function claimDeviceCode(port, forwardedFor) {
  const res = await fetch(`http://127.0.0.1:${port}/api/auth/device?user_code=ZZZZZZZZ`, {
    headers: forwardedFor ? { "x-forwarded-for": forwardedFor } : {},
  });
  await res.arrayBuffer();
  return res.status;
}

test("device code claims are limited per visitor, not shared by everyone behind the edge", async (t) => {
  const server = await serve(edge);
  t.after(() => server.close());
  const { port } = server.address();
  for (let i = 0; i < 30; i++) assert.notEqual(await claimDeviceCode(port, "198.51.100.7"), 429, `claim ${i}`);
  assert.equal(await claimDeviceCode(port, "198.51.100.7"), 429);
  assert.notEqual(await claimDeviceCode(port, "198.51.100.8"), 429, "another visitor can still sign in");
});

test("a direct visitor's forged address does not get them a fresh allowance", async (t) => {
  const server = await serve(trustedProxyList([]));
  t.after(() => server.close());
  const { port } = server.address();
  for (let i = 0; i < 30; i++) assert.notEqual(await claimDeviceCode(port, `198.51.100.${i}`), 429, `claim ${i}`);
  assert.equal(await claimDeviceCode(port, "198.51.100.200"), 429);
});
