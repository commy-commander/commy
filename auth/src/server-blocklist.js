import { createHash } from "node:crypto";
import { readFile, stat } from "node:fs/promises";

export const SERVER_BLOCKLIST_PATH = "/commy/server-blocklist";
export const LIST_PURPOSE = "server-blocklist";
export const LIST_VERSION = 1;
const LIST_DAYS = 7;
const RESIGN_MS = 24 * 60 * 60 * 1000;
const RECHECK_MS = 60 * 1000;
const CLIENT_CACHE_SECONDS = 300;
const DAY_MS = 24 * 60 * 60 * 1000;
const DOMAIN = /^(?=.{1,253}$)([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z][a-z0-9-]{0,61}[a-z0-9]$/;
const DATE = /^\d{4}-\d{2}-\d{2}$/;

export function blockedServerLine(line) {
  const text = line.split("#")[0].trim().toLowerCase();
  if (!text) return null;
  const [raw, date] = text.split(/\s+/);
  const domain = raw.replace(/^https?:\/\//, "").replace(/\/.*$/, "").replace(/\.$/, "");
  if (!DOMAIN.test(domain)) return null;
  const since = date && DATE.test(date) ? Date.parse(`${date}T00:00:00Z`) : 0;
  return { domain, since: Number.isFinite(since) ? since : 0 };
}

export function blockedServers(text) {
  const byDomain = new Map();
  for (const line of String(text ?? "").split("\n")) {
    const entry = blockedServerLine(line);
    if (entry && !byDomain.has(entry.domain)) byDomain.set(entry.domain, entry);
  }
  return [...byDomain.values()].sort((a, b) => a.domain.localeCompare(b.domain));
}

export function sealList({ listKey, origin, domains, homes, now }) {
  const list = JSON.stringify({ v: LIST_VERSION, issued: now, expires: now + LIST_DAYS * DAY_MS, domains, homes });
  return { list, key: listKey.published.id, sig: listKey.sign(LIST_PURPOSE, origin, list) };
}

export function createServerBlocklist({ listKey, origin, serverKey, file = null, log = console, now = () => Date.now() }) {
  const homes = [{ home: origin, key: serverKey.published }];
  let domains = [];
  let fileStamp = null;
  let served = null;
  let timer = null;

  function publish() {
    const body = JSON.stringify(sealList({ listKey, origin, domains, homes, now: now() }));
    const etag = `"${createHash("sha256").update(body).digest("hex").slice(0, 32)}"`;
    served = { body, etag, at: now() };
  }

  async function readFileIfChanged() {
    if (!file) return false;
    try {
      const info = await stat(file);
      const stamp = `${info.mtimeMs}:${info.size}`;
      if (stamp === fileStamp) return false;
      domains = blockedServers(await readFile(file, "utf8"));
      fileStamp = stamp;
      log.log(`[auth] server blocklist holds ${domains.length} servers`);
      return true;
    } catch (err) {
      if (fileStamp !== "missing") log.warn(`[auth] server blocklist ${file} unreadable: ${err.code ?? err.message}`);
      fileStamp = "missing";
      return false;
    }
  }

  async function refresh() {
    const changed = await readFileIfChanged();
    if (changed || !served || now() - served.at >= RESIGN_MS) publish();
  }

  function start() {
    const ready = refresh();
    timer = setInterval(() => refresh().catch(() => {}), RECHECK_MS);
    timer.unref?.();
    return ready;
  }

  function stop() {
    clearInterval(timer);
  }

  function blocks(domain) {
    const name = String(domain ?? "").toLowerCase().replace(/\.$/, "");
    return domains.some((entry) => name === entry.domain || name.endsWith(`.${entry.domain}`));
  }

  function respond(req, res) {
    if (!served) publish();
    const headers = {
      "content-type": "application/json",
      "access-control-allow-origin": "*",
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

  return { start, stop, refresh, blocks, respond, publicKey: listKey.published };
}
