import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { describeConnectionCheck, relayRoundTripText } = require("../scripts/diagnostics.js");

const check = (over = {}) => ({ quality: "good", direct: "open", relay_reachable: true, relay_rtt_ms: 42, ipv6: false, ...over });

test("a good open network reads in plain words", () => {
  const view = describeConnectionCheck(check());
  assert.equal(view.badge, "Good");
  assert.equal(view.tone, "good");
  assert.equal(view.direct, "Yes");
  assert.equal(view.relay, "42 ms");
  assert.match(view.detail, /connect straight to other people/);
});

test("relay-only networks say so and keep the round trip", () => {
  const view = describeConnectionCheck(check({ quality: "fair", direct: "blocked", relay_rtt_ms: 88.6 }));
  assert.equal(view.tone, "ok");
  assert.equal(view.direct, "No, using the relay");
  assert.equal(view.relay, "89 ms");
  assert.match(view.detail, /through Commy's relay/);
});

test("offline hides the direct answer and explains what to do", () => {
  const view = describeConnectionCheck(check({ quality: "offline", direct: "unchecked", relay_reachable: false, relay_rtt_ms: null }));
  assert.equal(view.badge, "Offline");
  assert.equal(view.tone, "bad");
  assert.equal(view.direct, "-");
  assert.equal(view.relay, "Not reachable");
  assert.equal(view.detail, "");
  assert.match(view.summary, /internet connection/);
});

test("browsers explain that they always use the relay", () => {
  const view = describeConnectionCheck(check({ direct: "unchecked" }), { browser: true });
  assert.equal(view.direct, "Browsers always use the relay");
  assert.match(view.detail, /web app always goes through/);
  assert.equal(describeConnectionCheck(check({ direct: "unchecked" })).direct, "Not checked");
  assert.equal(describeConnectionCheck(check(), { browser: true }).direct, "Yes");
});

test("unknown values fall back safely", () => {
  const view = describeConnectionCheck({ quality: "weird", direct: "odd" });
  assert.equal(view.badge, "Offline");
  assert.equal(describeConnectionCheck(null).badge, "Offline");
  assert.equal(relayRoundTripText({ relay_reachable: true, relay_rtt_ms: null }), "Not measured");
});

test("no wording names hosts, addresses, ports or the networking library", () => {
  for (const quality of ["good", "fair", "poor", "offline"]) {
    for (const direct of ["open", "limited", "blocked", "unchecked"]) {
      for (const browser of [false, true]) {
        const text = Object.values(describeConnectionCheck(check({ quality, direct }), { browser })).join(" ");
        assert.doesNotMatch(text, /iroh|https?:|\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b|port|NAT|UDP|QUIC/i);
      }
    }
  }
});
