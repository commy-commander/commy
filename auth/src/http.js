import { LINK_BLOCKLIST_PATH } from "./link-blocklist.js";

function sendJson(res, status, body) {
  res.writeHead(status, {
    "content-type": "application/json",
    "access-control-allow-origin": "*",
    "cache-control": "public, max-age=300",
  });
  res.end(body);
}

function endOnlyThisRequest(req, res, err) {
  if (!req.destroyed) console.warn(`[auth] ${req.method} ${req.url?.split("?")[0]} failed: ${err?.code ?? err?.message ?? err}`);
  if (res.headersSent || res.destroyed) res.destroy();
  else sendJson(res, 500, '{"error":"internal_error"}');
}

export function createRequestHandler({ authHandler, basePath, discovery, pool, redis, linkBlocklist = null }) {
  async function route(req, res) {
    const path = (req.url ?? "/").split("?")[0];

    if (path === "/healthz") {
      try {
        await pool.query("select 1");
      } catch {
        return sendJson(res, 503, '{"ok":false}');
      }
      const redisOk = redis ? await redis.ping().catch(() => false) : null;
      return sendJson(res, 200, JSON.stringify(redisOk === null ? { ok: true } : { ok: true, redis: redisOk }));
    }
    if (path === "/.well-known/commy-auth.json") {
      return sendJson(res, 200, discovery);
    }
    if (linkBlocklist?.enabled && path === `${basePath}${LINK_BLOCKLIST_PATH}` && (req.method === "GET" || req.method === "HEAD")) {
      return linkBlocklist.respond(req, res);
    }
    if (path.startsWith(basePath)) {
      return authHandler(req, res);
    }
    sendJson(res, 404, '{"error":"not_found"}');
  }

  return (req, res) => route(req, res).catch((err) => endOnlyThisRequest(req, res, err));
}
