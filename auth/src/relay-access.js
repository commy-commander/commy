import { createHash, timingSafeEqual } from "node:crypto";
import { normalizeNodeId } from "./devices.js";

export const RELAY_ACCESS_PATH = "/internal/relay-access";
export const ENDPOINT_HEADER = "x-iroh-endpoint-id";
const MIN_TOKEN_LENGTH = 32;

function digest(value) {
  return createHash("sha256").update(String(value)).digest();
}

function sameSecret(given, expected) {
  return timingSafeEqual(digest(given), digest(expected));
}

function bearer(req) {
  const header = req.headers.authorization ?? "";
  return header.startsWith("Bearer ") ? header.slice("Bearer ".length).trim() : "";
}

function reply(res, status, body) {
  res.writeHead(status, { "content-type": "application/json", "cache-control": "no-store" });
  res.end(JSON.stringify(body));
}

export function relayAccessToken(env) {
  const token = (env.RELAY_ACCESS_TOKEN ?? "").trim();
  return token.length >= MIN_TOKEN_LENGTH ? token : null;
}

export function linkedDeviceLookup(authContext) {
  return async (nodeId) => {
    const { adapter } = await authContext;
    const row = await adapter.findOne({ model: "linkedDevice", where: [{ field: "nodeId", value: nodeId }] });
    return Boolean(row);
  };
}

export function createRelayAccess({ token, isLinked }) {
  return {
    enabled: Boolean(token),
    handles(req) {
      return (req.url ?? "").split("?")[0] === RELAY_ACCESS_PATH;
    },
    async respond(req, res) {
      if (req.method !== "POST") return reply(res, 405, { error: "method_not_allowed" });
      if (!sameSecret(bearer(req), token)) return reply(res, 401, { error: "unauthorized" });
      const nodeId = normalizeNodeId(req.headers[ENDPOINT_HEADER]);
      if (!nodeId) return reply(res, 400, { error: "bad_endpoint_id" });
      return reply(res, 200, { linked: await isLinked(nodeId) });
    },
  };
}
