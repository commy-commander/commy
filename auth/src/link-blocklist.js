import { createHash } from "node:crypto";

export const DEFAULT_LINK_BLOCKLIST_SOURCES = [
  "https://malware-filter.gitlab.io/malware-filter/phishing-filter-domains.txt",
  "https://malware-filter.gitlab.io/malware-filter/urlhaus-filter-domains.txt",
];
export const LINK_BLOCKLIST_PATH = "/commy/link-blocklist";

const REFRESH_MS = 12 * 60 * 60 * 1000;
const RETRY_MS = 15 * 60 * 1000;
const SOURCE_TIMEOUT_MS = 60_000;
const MAX_SOURCE_BYTES = 16 * 1024 * 1024;
const MAX_DOMAINS = 500_000;
const MAX_DOMAIN_LENGTH = 253;
const CLIENT_CACHE_SECONDS = 60 * 60;
const NOT_READY_RETRY_SECONDS = 300;
const DOMAIN_CHARS = /^[a-z0-9._-]+$/;

export function listedDomain(line) {
  const trimmed = line.trim();
  if (!trimmed || trimmed.startsWith("#") || trimmed.startsWith("!")) return null;
  const token = trimmed.split(/\s+/).pop();
  const domain = token.replace(/^\|\|/, "").replace(/\^$/, "").replace(/^\.+|\.+$/g, "").toLowerCase();
  const plausible = domain.includes(".") && domain.length <= MAX_DOMAIN_LENGTH && DOMAIN_CHARS.test(domain);
  return plausible ? domain : null;
}

export function blocklistDomains(text) {
  const domains = new Set();
  for (const line of String(text).split("\n")) {
    const domain = listedDomain(line);
    if (domain) domains.add(domain);
  }
  return domains;
}

export function linkBlocklistSources(raw) {
  const value = (raw ?? "").trim();
  if (value === "off") return [];
  if (!value) return DEFAULT_LINK_BLOCKLIST_SOURCES;
  return value
    .split(",")
    .map((s) => s.trim())
    .filter((s) => /^https:\/\/\S+$/.test(s));
}

async function readSource(doFetch, url) {
  const res = await doFetch(url, { redirect: "follow", signal: AbortSignal.timeout(SOURCE_TIMEOUT_MS) });
  if (!res.ok) throw new Error(`answered ${res.status}`);
  if (Number(res.headers.get("content-length") ?? 0) > MAX_SOURCE_BYTES) throw new Error("list too large");
  const text = await res.text();
  if (text.length > MAX_SOURCE_BYTES) throw new Error("list too large");
  return blocklistDomains(text);
}

function joinLists(lists) {
  const all = new Set();
  for (const domains of lists) {
    for (const domain of domains) {
      if (all.size >= MAX_DOMAINS) return all;
      all.add(domain);
    }
  }
  return all;
}

export function createLinkBlocklist({ sources, fetch: doFetch = fetch, log = console } = {}) {
  const latest = new Map();
  let served = null;
  let timer = null;

  function publish() {
    const domains = [...joinLists(latest.values())].sort();
    if (domains.length === 0) return;
    const body = `${domains.join("\n")}\n`;
    const etag = `"${createHash("sha256").update(body).digest("hex").slice(0, 32)}"`;
    served = { body, etag, count: domains.length };
  }

  async function refresh() {
    const results = await Promise.allSettled(sources.map((url) => readSource(doFetch, url)));
    let failed = 0;
    results.forEach((result, i) => {
      if (result.status === "fulfilled" && result.value.size > 0) latest.set(sources[i], result.value);
      else {
        failed += 1;
        log.warn(`[auth] link blocklist from ${new URL(sources[i]).hostname} failed: ${result.reason?.message ?? "empty list"}`);
      }
    });
    publish();
    return failed === 0;
  }

  function schedule(ms) {
    timer = setTimeout(async () => {
      const ok = await refresh().catch(() => false);
      schedule(ok ? REFRESH_MS : RETRY_MS);
    }, ms);
    timer.unref?.();
  }

  function respond(req, res) {
    if (!served) {
      res.writeHead(503, { "content-type": "text/plain", "retry-after": String(NOT_READY_RETRY_SECONDS) });
      return res.end();
    }
    const headers = {
      "content-type": "text/plain; charset=utf-8",
      "cache-control": `public, max-age=${CLIENT_CACHE_SECONDS}`,
      etag: served.etag,
      "x-content-type-options": "nosniff",
    };
    if (req.headers["if-none-match"] === served.etag) {
      res.writeHead(304, headers);
      return res.end();
    }
    res.writeHead(200, headers);
    res.end(req.method === "HEAD" ? undefined : served.body);
  }

  return {
    enabled: sources.length > 0,
    refresh,
    respond,
    start() {
      if (sources.length > 0) schedule(0);
    },
    stop() {
      clearTimeout(timer);
    },
    count: () => served?.count ?? 0,
  };
}
