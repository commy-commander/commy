const GB = 1e9;

export function parseMetrics(text) {
  const read = (name) => {
    const m = new RegExp(`^relayserver_${name}_total\\s+([0-9.e+]+)\\s*$`, "m").exec(text);
    return m ? Number(m[1]) : null;
  };
  const sent = read("bytes_sent");
  const recv = read("bytes_recv");
  if (sent === null || recv === null || !Number.isFinite(sent) || !Number.isFinite(recv)) return null;
  return { sent, recv };
}

export function monthKey(date) {
  return `${date.getUTCFullYear()}-${String(date.getUTCMonth() + 1).padStart(2, "0")}`;
}

export function initialState(now) {
  return { month: monthKey(now), total: 0, last: null, level: "ok" };
}

export function step(state, sample, now) {
  const month = monthKey(now);
  const next = { ...state };
  if (next.month !== month) {
    next.month = month;
    next.total = 0;
  }
  if (sample) {
    const delta = (cur, prev) => (prev === undefined || prev === null ? 0 : cur >= prev ? cur - prev : cur);
    next.total += delta(sample.sent, state.last?.sent) + delta(sample.recv, state.last?.recv);
    next.last = sample;
  }
  return next;
}

export function levelFor(total, limitBytes, warnPercent, pausePercent) {
  const pct = limitBytes > 0 ? (total / limitBytes) * 100 : 0;
  if (pct >= pausePercent) return "paused";
  if (pct >= warnPercent) return "warn";
  return "ok";
}

export function describe(state, limitBytes) {
  const used = (state.total / GB).toFixed(1);
  const limit = (limitBytes / GB).toFixed(0);
  const pct = limitBytes > 0 ? ((state.total / limitBytes) * 100).toFixed(1) : "0";
  const what = {
    ok: "is back under its limit and accepting new connections",
    warn: "is getting close to its monthly limit",
    paused: "reached its monthly limit and stopped accepting new connections; clients fall back to other relays until next month",
  }[state.level];
  return `Commy relay ${what}: ${used} GB of ${limit} GB (${pct}%) used in ${state.month}.`;
}

export function loadConfig(env) {
  const num = (v, d) => (v !== undefined && v !== "" && Number.isFinite(Number(v)) ? Number(v) : d);
  return {
    metricsUrl: env.RELAY_METRICS_URL || "http://relay:9090/metrics",
    limitBytes: num(env.RELAY_MONTHLY_LIMIT_GB, 6000) * GB,
    warnPercent: num(env.RELAY_WARN_PERCENT, 75),
    pausePercent: num(env.RELAY_PAUSE_PERCENT, 90),
    intervalMs: num(env.RELAY_METER_INTERVAL_SECONDS, 60) * 1000,
    webhookUrl: env.ALERT_WEBHOOK_URL || "",
    stateFile: env.STATE_FILE || "/data/usage.json",
    flagFile: env.FLAG_FILE || "/flags/relay-paused",
  };
}

export function fixUpgradeHead(head) {
  if (!/^HTTP\/1\.[01] 101\b/.test(head) || /\r\nconnection:/i.test(head)) return head;
  const end = head.indexOf("\r\n\r\n");
  if (end < 0) return head;
  return `${head.slice(0, end)}\r\nConnection: Upgrade${head.slice(end)}`;
}

export function requestHead(method, url, rawHeaders) {
  let out = `${method} ${url} HTTP/1.1\r\n`;
  for (let i = 0; i + 1 < rawHeaders.length; i += 2) {
    const name = rawHeaders[i];
    const value = String(rawHeaders[i + 1]).replace(/[\r\n]/g, "");
    if (/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(name)) out += `${name}: ${value}\r\n`;
  }
  return `${out}\r\n`;
}
