import { createHash } from "node:crypto";
import { listEntry } from "./listing.js";

export const SERVER_LIST_PATH = "/commy/server-list";
export const LIST_PURPOSE = "server-list";
export const LIST_VERSION = 1;
const LIST_DAYS = 7;
const DAY_MS = 24 * 60 * 60 * 1000;
const RESIGN_MS = DAY_MS;
const CLIENT_CACHE_SECONDS = 300;
const RELOAD_MS = 10 * 60 * 1000;

export function sealServerList({ listKey, origin, servers, now }) {
  const list = JSON.stringify({ v: LIST_VERSION, issued: now, expires: now + LIST_DAYS * DAY_MS, servers });
  return { list, key: listKey.published.id, sig: listKey.sign(LIST_PURPOSE, origin, list) };
}

export function entriesFrom(rows, blocked = () => false) {
  return rows
    .filter((row) => !blocked(new URL(row.origin).hostname))
    .map(listEntry)
    .sort((a, b) => a.origin.localeCompare(b.origin));
}

export function createServerList({ listKey, origin, loadRows = async () => [], blocked = () => false, log = console, now = () => Date.now() }) {
  let served = null;
  let timer = null;

  function publish(servers) {
    const body = JSON.stringify(sealServerList({ listKey, origin, servers, now: now() }));
    const etag = `"${createHash("sha256").update(body).digest("hex").slice(0, 32)}"`;
    served = { body, etag, at: now(), servers };
  }

  async function refresh() {
    try {
      const servers = entriesFrom(await loadRows(), blocked);
      const same = served && JSON.stringify(servers) === JSON.stringify(served.servers);
      if (!same || now() - served.at >= RESIGN_MS) publish(servers);
    } catch (err) {
      log.warn(`[auth] server list not refreshed: ${err.message}`);
      if (!served) publish([]);
    }
  }

  function start() {
    const ready = refresh();
    timer = setInterval(refresh, RELOAD_MS);
    timer.unref?.();
    return ready;
  }

  function stop() {
    clearInterval(timer);
  }

  function respond(req, res) {
    if (!served) publish([]);
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

  return { start, stop, refresh, respond, publicKey: listKey.published, servers: () => served?.servers ?? [] };
}
