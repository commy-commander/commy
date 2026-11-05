const KIB = 1024;
const MIB = 1024 * KIB;
const TAG_BYTES = 16;
const LENGTH_PREFIX_BYTES = 4;
export const MAX_FILE_PLAIN = 30 * MIB;
export const SEALED_SIZES = [64 * KIB, 256 * KIB, MIB, 2 * MIB, 4 * MIB, 8 * MIB, 12 * MIB, 16 * MIB, 24 * MIB, MAX_FILE_PLAIN + LENGTH_PREFIX_BYTES].map(size => size + TAG_BYTES);
const FILE_ID = /^\/([0-9a-f]{64})$/;
const KEPT_FOR = "public, max-age=31536000, immutable";

const CORS = {
  "access-control-allow-origin": "*",
  "access-control-allow-methods": "GET, HEAD, PUT, OPTIONS",
  "access-control-allow-headers": "content-type",
  "access-control-max-age": "86400",
};

function reply(status, body = null, headers = {}) {
  return new Response(body, { status, headers: { ...CORS, ...headers } });
}

export function objectName(id) {
  return `f/${id}`;
}

export function fitsSealed(length) {
  return SEALED_SIZES.includes(length);
}

async function check(bucket, id) {
  const head = await bucket.head(objectName(id));
  return head ? reply(200, null, { "content-length": String(head.size) }) : reply(404);
}

async function fetchFile(bucket, id) {
  const object = await bucket.get(objectName(id));
  if (!object) return reply(404, "no such file", { "cache-control": "no-store" });
  return reply(200, object.body, {
    "content-type": "application/octet-stream",
    "content-length": String(object.size),
    "cache-control": KEPT_FOR,
  });
}

async function keepFile(bucket, id, request) {
  const declared = Number(request.headers.get("content-length"));
  if (!fitsSealed(declared) || !request.body) return reply(400, "not a sealed file");
  if (await bucket.head(objectName(id))) return reply(204);
  const kept = await bucket.put(objectName(id), request.body, { httpMetadata: { contentType: "application/octet-stream" } });
  if (!kept || kept.size !== declared) {
    await bucket.delete(objectName(id));
    return reply(400, "not a sealed file");
  }
  return reply(204);
}

export async function handle(request, env) {
  const bucket = env.FILES;
  if (!bucket) return reply(503, "this worker has no FILES bucket bound");
  const path = new URL(request.url).pathname.replace(/\/+$/, "") || "/";
  const method = request.method;
  if (method === "OPTIONS") return reply(204);
  if (path === "/") {
    if (method !== "GET" && method !== "HEAD") return reply(405);
    return reply(200, JSON.stringify({ commy_files: 1, max_bytes: MAX_FILE_PLAIN }), { "content-type": "application/json", "cache-control": "no-store" });
  }
  const id = FILE_ID.exec(path)?.[1];
  if (!id) return reply(404);
  if (method === "HEAD") return check(bucket, id);
  if (method === "GET") return fetchFile(bucket, id);
  if (method === "PUT") return keepFile(bucket, id, request);
  return reply(405);
}

export default { fetch: handle };
