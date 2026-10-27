import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { createSignalTracker, linkQuality, worstLinkQuality, SIGNAL_MIN_SAMPLE_PACKETS } = require("../scripts/voice-signal.js");

const link = (lost, sent, rtt = 40) => ({ user: "a", rtt_ms: rtt, loss_pct: (lost * 100) / Math.max(1, sent), lost_packets: lost, sent_packets: sent });

test("loss from long ago does not keep a healthy link marked poor", () => {
  const tracker = createSignalTracker();
  const [first] = tracker.read([link(900, 10000)]);
  assert.equal(first.quality, "good");
  const [later] = tracker.read([link(900, 10400)]);
  assert.equal(later.loss_pct, 0);
  assert.equal(later.quality, "good");
});

test("loss happening now shows up within one reading", () => {
  const tracker = createSignalTracker();
  tracker.read([link(0, 1000)]);
  const [now] = tracker.read([link(40, 1400)]);
  assert.equal(now.loss_pct, 10);
  assert.equal(now.quality, "bad");
});

test("a reading with too few packets keeps the last loss instead of guessing", () => {
  const tracker = createSignalTracker();
  tracker.read([link(0, 1000)]);
  tracker.read([link(40, 1400)]);
  const [quiet] = tracker.read([link(40, 1400 + SIGNAL_MIN_SAMPLE_PACKETS - 1)]);
  assert.equal(quiet.quality, "bad");
  const [enough] = tracker.read([link(40, 1400 + SIGNAL_MIN_SAMPLE_PACKETS * 4)]);
  assert.equal(enough.quality, "good");
});

test("a reconnect that restarts the counters starts a fresh baseline", () => {
  const tracker = createSignalTracker();
  tracker.read([link(10, 5000)]);
  const [restarted] = tracker.read([link(0, 20)]);
  assert.equal(restarted.quality, "good");
  const [after] = tracker.read([link(0, 400)]);
  assert.equal(after.loss_pct, 0);
});

test("ping alone still marks a slow link", () => {
  const tracker = createSignalTracker();
  const [slow] = tracker.read([link(0, 100, 300)]);
  assert.equal(slow.quality, "bad");
  assert.equal(linkQuality(150, 0), "ok");
});

test("links without packet counts fall back to the reported loss", () => {
  const tracker = createSignalTracker();
  const [old] = tracker.read([{ user: "b", rtt_ms: 30, loss_pct: 2.3 }]);
  assert.equal(old.quality, "ok");
});

test("the signal shows the worst link, and an empty call is good", () => {
  assert.equal(worstLinkQuality([]), "good");
  assert.equal(worstLinkQuality([{ quality: "good" }, { quality: "ok" }]), "ok");
  assert.equal(worstLinkQuality([{ quality: "ok" }, { quality: "bad" }, { quality: "good" }]), "bad");
});

test("people who leave are forgotten", () => {
  const tracker = createSignalTracker();
  tracker.read([link(0, 1000)]);
  tracker.read([link(40, 1400)]);
  tracker.read([]);
  const [back] = tracker.read([link(40, 1400)]);
  assert.equal(back.quality, "good");
});
