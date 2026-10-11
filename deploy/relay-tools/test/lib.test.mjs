import assert from "node:assert/strict";
import { test } from "node:test";
import { describe, initialState, levelFor, loadConfig, monthKey, parseMetrics, step } from "../lib.mjs";

const METRICS = `# HELP relayserver_bytes_sent Number of bytes sent.
# TYPE relayserver_bytes_sent counter
relayserver_bytes_sent_total 1500
relayserver_bytes_recv_total 2500
relayserver_send_packets_sent_total 3
`;

test("parses relay byte counters", () => {
  assert.deepEqual(parseMetrics(METRICS), { sent: 1500, recv: 2500 });
  assert.deepEqual(parseMetrics("relayserver_bytes_sent_total 1.5e+10\nrelayserver_bytes_recv_total 0\n"), { sent: 1.5e10, recv: 0 });
  assert.equal(parseMetrics("relayserver_bytes_sent_total 1\n"), null);
  assert.equal(parseMetrics(""), null);
});

test("first sample is a baseline, later samples add deltas", () => {
  const t = new Date("2026-10-11T00:00:00Z");
  let s = initialState(t);
  s = step(s, { sent: 1000, recv: 1000 }, t);
  assert.equal(s.total, 0);
  s = step(s, { sent: 1500, recv: 1200 }, t);
  assert.equal(s.total, 700);
});

test("a relay restart counts the new counter values", () => {
  const t = new Date("2026-10-11T00:00:00Z");
  let s = { ...initialState(t), total: 100, last: { sent: 5000, recv: 5000 } };
  s = step(s, { sent: 40, recv: 60 }, t);
  assert.equal(s.total, 200);
});

test("a missed sample changes nothing", () => {
  const t = new Date("2026-10-11T00:00:00Z");
  const s = { ...initialState(t), total: 100, last: { sent: 1, recv: 1 } };
  assert.deepEqual(step(s, null, t), s);
});

test("totals reset on the first of the month (UTC)", () => {
  let s = { month: "2026-10", total: 9e12, last: { sent: 10, recv: 10 }, level: "paused" };
  s = step(s, { sent: 30, recv: 10 }, new Date("2026-11-01T00:00:30Z"));
  assert.equal(s.month, "2026-11");
  assert.equal(s.total, 20);
  assert.equal(monthKey(new Date("2026-12-31T23:59:59Z")), "2026-12");
});

test("levels follow the warn and pause thresholds", () => {
  const limit = 1000;
  assert.equal(levelFor(0, limit, 75, 90), "ok");
  assert.equal(levelFor(749, limit, 75, 90), "ok");
  assert.equal(levelFor(750, limit, 75, 90), "warn");
  assert.equal(levelFor(900, limit, 75, 90), "paused");
  assert.equal(levelFor(5000, 0, 75, 90), "ok", "a zero limit never pauses");
});

test("alert text states usage plainly", () => {
  const msg = describe({ month: "2026-10", total: 4.5e12, level: "warn" }, 6e12);
  assert.equal(msg, "Commy relay is getting close to its monthly limit: 4500.0 GB of 6000 GB (75.0%) used in 2026-10.");
});

test("config defaults and overrides", () => {
  const d = loadConfig({});
  assert.equal(d.limitBytes, 6000e9);
  assert.equal(d.pausePercent, 90);
  assert.equal(d.intervalMs, 60000);
  const c = loadConfig({ RELAY_MONTHLY_LIMIT_GB: "100", RELAY_PAUSE_PERCENT: "95", RELAY_METER_INTERVAL_SECONDS: "abc" });
  assert.equal(c.limitBytes, 100e9);
  assert.equal(c.pausePercent, 95);
  assert.equal(c.intervalMs, 60000);
});
