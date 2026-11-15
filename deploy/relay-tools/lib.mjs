export const GB = 1e9;

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

function bytesSince(current, previous) {
  if (previous === undefined || previous === null) return 0;
  const relayRestartedAndResetItsCounter = current < previous;
  return relayRestartedAndResetItsCounter ? current : current - previous;
}

export function step(state, sample, now) {
  const month = monthKey(now);
  const next = { ...state };
  if (next.month !== month) {
    next.month = month;
    next.total = 0;
  }
  if (sample) {
    next.total += bytesSince(sample.sent, state.last?.sent) + bytesSince(sample.recv, state.last?.recv);
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

export const ENDPOINT_ID = /^[0-9a-f]{64}$/;
export const LINKED_KEEP_MS = 10 * 60 * 1000;
export const STRANGER_KEEP_MS = 5 * 1000;

export function accessMode(value) {
  return String(value ?? "").trim().toLowerCase() === "enforce" ? "enforce" : "watch";
}

export function letIn(mode, linked) {
  return mode === "watch" || linked !== false;
}

export class Answers {
  constructor(max = 50_000) {
    this.max = max;
    this.kept = new Map();
  }

  get(endpoint, now) {
    const kept = this.kept.get(endpoint);
    if (!kept) return undefined;
    if (now >= kept.until) {
      this.kept.delete(endpoint);
      return undefined;
    }
    return kept.linked;
  }

  set(endpoint, linked, now) {
    this.kept.delete(endpoint);
    this.kept.set(endpoint, { linked, until: now + (linked ? LINKED_KEEP_MS : STRANGER_KEEP_MS) });
    while (this.kept.size > this.max) this.kept.delete(this.kept.keys().next().value);
  }
}

export function dayKey(date) {
  return date.toISOString().slice(0, 10);
}

export function emptyTally(now) {
  return { day: dayKey(now), endpoints: 0, strangers: 0, refused: 0, unchecked: 0, seen: new Set(), outsiders: new Set() };
}

export function tally(state, now, endpoint, outcome) {
  const next = state.day === dayKey(now) ? state : emptyTally(now);
  if (!next.seen.has(endpoint)) {
    next.seen.add(endpoint);
    next.endpoints = next.seen.size;
  }
  if (outcome.linked === false && !next.outsiders.has(endpoint)) {
    next.outsiders.add(endpoint);
    next.strangers = next.outsiders.size;
  }
  if (!outcome.allowed) next.refused += 1;
  if (outcome.linked === undefined) next.unchecked += 1;
  return next;
}

export function tallyReport(state, mode) {
  const { day, endpoints, strangers, refused, unchecked } = state;
  return { mode, day, endpoints, strangers, refused, unchecked };
}

export const DEFAULT_AUTH_URL = "http://auth:3001/internal/relay-access";

export function authUrls(value) {
  const urls = String(value ?? "")
    .split(",")
    .map((url) => url.trim())
    .filter((url) => /^https?:\/\/[^\s]+$/.test(url));
  return urls.length ? urls : [DEFAULT_AUTH_URL];
}

export function combineAnswers(answers) {
  if (answers.includes(true)) return true;
  if (answers.length && answers.every((answer) => answer === false)) return false;
  return undefined;
}

export function loadAccessConfig(env) {
  const num = (v, d) => (v !== undefined && v !== "" && Number.isFinite(Number(v)) ? Number(v) : d);
  return {
    mode: accessMode(env.RELAY_ACCESS_MODE),
    authUrls: authUrls(env.RELAY_ACCESS_AUTH_URLS),
    token: (env.RELAY_ACCESS_TOKEN || "").trim(),
    port: num(env.ACCESS_PORT, 3342),
    timeoutMs: num(env.RELAY_ACCESS_TIMEOUT_MS, 2000),
    stateFile: env.ACCESS_STATE_FILE || "/data/access.json",
  };
}
