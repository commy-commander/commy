import assert from "node:assert/strict";
import http from "node:http";
import net from "node:net";
import { after, before, test } from "node:test";
import { setTimeout as delay } from "node:timers/promises";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { toNodeHandler } from "better-auth/node";
import { buildAuthOptions } from "../src/auth.js";
import { loadConfig } from "../src/config.js";
import { createRequestHandler } from "../src/http.js";

const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: "http://localhost:3001" };
const TOKEN = "ab".repeat(16);

const unhandled = [];
const recordUnhandled = (err) => unhandled.push(err);
let server;
let port;

function listen(authHandler) {
  const config = loadConfig(ENV);
  const handler = createRequestHandler({
    authHandler,
    basePath: config.basePath,
    discovery: "{}",
    pool: { query: async () => {} },
    redis: null,
  });
  const s = http.createServer(handler);
  return new Promise((resolve) => s.listen(0, "127.0.0.1", () => resolve(s)));
}

before(async () => {
  process.on("unhandledRejection", recordUnhandled);
  process.on("uncaughtException", recordUnhandled);
  const options = buildAuthOptions(loadConfig(ENV), { pool: null, send: async () => {} });
  options.database = sealedAtRest(memoryAdapter({
    user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], inviteBackground: [], doorbellRing: [],
  }), options.secret);
  options.rateLimit = { enabled: false };
  server = await listen(toNodeHandler(betterAuth(options)));
  port = server.address().port;
});

after(() => {
  server.close();
  process.off("unhandledRejection", recordUnhandled);
  process.off("uncaughtException", recordUnhandled);
});

async function hangUpMidRequest(path, { declaredLength = 200, sent = '{"tok', waitMs = 100 } = {}) {
  const socket = net.connect(port, "127.0.0.1");
  await new Promise((resolve, reject) => socket.once("connect", resolve).once("error", reject));
  socket.write(
    `POST ${path} HTTP/1.1\r\nHost: localhost:3001\r\nContent-Type: application/json\r\nContent-Length: ${declaredLength}\r\n\r\n${sent}`,
  );
  await delay(waitMs);
  socket.destroy();
  await delay(100);
}

async function post(path, body) {
  const res = await fetch(`http://127.0.0.1:${port}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json", origin: "http://localhost:3001" },
    body: JSON.stringify(body),
  });
  return { status: res.status, json: await res.json() };
}

async function assertStillServing() {
  assert.deepEqual(unhandled, [], "a client hang-up must not reach the process");
  assert.equal((await fetch(`http://127.0.0.1:${port}/healthz`)).status, 200);
  const ids = ["cd".repeat(12)];
  assert.equal((await post("/api/auth/commy/doorbell/ring", { token: TOKEN, ids })).status, 200);
  assert.deepEqual((await post("/api/auth/commy/doorbell/take", { token: TOKEN })).json, { ids, hello: false });
}

test("a client that hangs up mid-body on the doorbell ends only its own request", async () => {
  await hangUpMidRequest("/api/auth/commy/doorbell/take");
  await hangUpMidRequest("/api/auth/commy/doorbell/ring");
  await assertStillServing();
});

test("a client that hangs up mid-body on sign-in ends only its own request", async () => {
  await hangUpMidRequest("/api/auth/sign-in/email");
  await assertStillServing();
});

test("a client that hangs up during a doorbell long-poll ends only its own request", async () => {
  const body = JSON.stringify({ token: "ef".repeat(16), wait: 2 });
  await hangUpMidRequest("/api/auth/commy/doorbell/take", { declaredLength: Buffer.byteLength(body), sent: body, waitMs: 300 });
  await assertStillServing();
});

test("a handler that throws gets a 500 and the server keeps serving", async () => {
  const failing = await listen(async () => {
    throw new Error("boom");
  });
  try {
    const res = await fetch(`http://127.0.0.1:${failing.address().port}/api/auth/anything`);
    assert.equal(res.status, 500);
    assert.deepEqual(await res.json(), { error: "internal_error" });
    assert.equal((await fetch(`http://127.0.0.1:${failing.address().port}/healthz`)).status, 200);
    assert.deepEqual(unhandled, []);
  } finally {
    failing.close();
  }
});
