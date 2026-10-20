import assert from "node:assert/strict";
import { test } from "node:test";
import { fixUpgradeHead, requestHead } from "../lib.mjs";

const RELAY_101 = "HTTP/1.1 101 Switching Protocols\r\nupgrade: iroh derp http\r\ndate: x\r\n\r\n";

test("adds Connection: Upgrade to a 101 that lacks it", () => {
  assert.equal(fixUpgradeHead(RELAY_101), "HTTP/1.1 101 Switching Protocols\r\nupgrade: iroh derp http\r\ndate: x\r\nConnection: Upgrade\r\n\r\n");
});

test("leaves other responses alone", () => {
  const ok = "HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: websocket\r\n\r\n";
  assert.equal(fixUpgradeHead(ok), ok);
  const plain = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
  assert.equal(fixUpgradeHead(plain), plain);
  assert.equal(fixUpgradeHead("HTTP/1.1 101 x\r\nupgrade: y"), "HTTP/1.1 101 x\r\nupgrade: y");
});

test("keeps the websocket subprotocol browsers need", () => {
  const browser = "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nsec-websocket-protocol: iroh-relay-v1\r\n\r\n";
  assert.match(fixUpgradeHead(browser), /\r\nsec-websocket-protocol: iroh-relay-v1\r\n/);
  const head = requestHead("GET", "/relay", ["Upgrade", "websocket", "Sec-WebSocket-Protocol", "iroh-relay-v1"]);
  assert.match(head, /\r\nSec-WebSocket-Protocol: iroh-relay-v1\r\n/);
});

test("rebuilds request heads and drops unsafe header names and newlines", () => {
  const head = requestHead("GET", "/relay", ["Host", "relay.example", "Upgrade", "iroh derp http", "Bad Name", "x", "X-Evil", "a\r\nInjected: 1"]);
  assert.equal(head, "GET /relay HTTP/1.1\r\nHost: relay.example\r\nUpgrade: iroh derp http\r\nX-Evil: aInjected: 1\r\n\r\n");
});
