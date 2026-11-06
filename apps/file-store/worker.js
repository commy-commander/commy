const KIB = 1024;
const MIB = 1024 * KIB;
const TAG_BYTES = 16;
const LENGTH_PREFIX_BYTES = 4;
export const MAX_FILE_PLAIN = 30 * MIB;
export const SEALED_SIZES = [64 * KIB, 256 * KIB, MIB, 2 * MIB, 4 * MIB, 8 * MIB, 12 * MIB, 16 * MIB, 24 * MIB, MAX_FILE_PLAIN + LENGTH_PREFIX_BYTES].map(size => size + TAG_BYTES);
export const SIGNED_FOR_MS = 5 * 60 * 1000;
export const DEFAULT_DAILY_UPLOAD_MB = 500;
export const MAX_ROSTER_BYTES = 2 * MIB;
export const MAX_ROSTER_KEYS = 20000;
export const MIN_SETUP_CODE = 16;
const REQUEST_DOMAIN = "commy-files/v2";
const ROSTER_DOMAIN = "commy-files-roster/v2";
const ROSTER_OBJECT = "_commy/roster.json";
const ROSTER_FRESH_MS = 15 * 1000;
const FILE_ID = /^\/([0-9a-f]{64})$/;
const KEY_HEX = /^[0-9a-f]{64}$/;
const SIG_HEX = /^[0-9a-f]{128}$/;
const KEPT_FOR = "private, max-age=31536000, immutable";

const CORS = {
  "access-control-allow-origin": "*",
  "access-control-allow-methods": "GET, HEAD, PUT, POST, OPTIONS",
  "access-control-allow-headers": "content-type, x-commy-key, x-commy-at, x-commy-sig, x-commy-setup",
  "access-control-max-age": "86400",
};

let rosterCache = null;

export function forgetRoster() {
  rosterCache = null;
}

function reply(status, body = null, headers = {}) {
  return new Response(body, { status, headers: { ...CORS, ...headers } });
}

function json(status, value) {
  return reply(status, JSON.stringify(value), { "content-type": "application/json", "cache-control": "no-store" });
}

export function objectName(id) {
  return `f/${id}`;
}

export function usageName(key) {
  return `u/${key}`;
}

export function fitsSealed(length) {
  return SEALED_SIZES.includes(length);
}

function bytesOfHex(hex) {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

export async function verifies(keyHex, message, sigHex) {
  if (!KEY_HEX.test(keyHex || "") || !SIG_HEX.test(sigHex || "")) return false;
  try {
    const key = await crypto.subtle.importKey("raw", bytesOfHex(keyHex), { name: "Ed25519" }, false, ["verify"]);
    return await crypto.subtle.verify({ name: "Ed25519" }, key, bytesOfHex(sigHex), new TextEncoder().encode(message));
  } catch {
    return false;
  }
}

export function requestMessage(method, target, at, length) {
  return `${REQUEST_DOMAIN}\n${method}\n${target}\n${at}\n${length}`;
}

export function rosterMessage(roster) {
  return `${ROSTER_DOMAIN}\n${roster}`;
}

function keyList(list) {
  return Array.isArray(list) && list.length <= MAX_ROSTER_KEYS && list.every(key => KEY_HEX.test(key));
}

export function parseRoster(text) {
  let roster;
  try {
    roster = JSON.parse(text);
  } catch {
    return null;
  }
  if (!roster || typeof roster !== "object" || !KEY_HEX.test(roster.owner || "")) return null;
  if (!keyList(roster.managers) || !keyList(roster.members) || !Number.isSafeInteger(roster.at)) return null;
  return roster;
}

function sameKeys(a, b) {
  return a.length === b.length && [...a].sort().join() === [...b].sort().join();
}

function canManage(roster, key) {
  return roster.owner === key || roster.managers.includes(key);
}

function admits(roster, key) {
  return canManage(roster, key) || roster.members.includes(key);
}

async function loadRoster(bucket, now) {
  if (rosterCache && now - rosterCache.read < ROSTER_FRESH_MS) return rosterCache.roster;
  const object = await bucket.get(ROSTER_OBJECT);
  const roster = object ? parseRoster(await object.text()) : null;
  rosterCache = { roster, read: now };
  return roster;
}

async function keepRoster(bucket, roster, now) {
  await bucket.put(ROSTER_OBJECT, JSON.stringify(roster), { httpMetadata: { contentType: "application/json" } });
  rosterCache = { roster, read: now };
}

async function signer(request, method, target, length, now) {
  const key = request.headers.get("x-commy-key");
  const at = Number(request.headers.get("x-commy-at"));
  if (!Number.isSafeInteger(at) || Math.abs(now - at) > SIGNED_FOR_MS) return null;
  const sig = request.headers.get("x-commy-sig");
  return (await verifies(key, requestMessage(method, target, at, length), sig)) ? key : null;
}

async function readSigned(request) {
  const declared = Number(request.headers.get("content-length"));
  if (!Number.isFinite(declared) || declared > MAX_ROSTER_BYTES) return null;
  const text = await request.text();
  if (text.length > MAX_ROSTER_BYTES) return null;
  let signed;
  try {
    signed = JSON.parse(text);
  } catch {
    return null;
  }
  if (!signed || typeof signed.roster !== "string") return null;
  if (!(await verifies(signed.key, rosterMessage(signed.roster), signed.sig))) return null;
  const roster = parseRoster(signed.roster);
  return roster ? { roster, key: signed.key } : null;
}

async function digest(text) {
  const hash = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(hash)].map(b => b.toString(16).padStart(2, "0")).join("");
}

async function setupMatches(env, given) {
  const code = typeof env.SETUP_CODE === "string" ? env.SETUP_CODE.trim() : "";
  if (code.length < MIN_SETUP_CODE || typeof given !== "string") return false;
  return (await digest(code)) === (await digest(given.trim()));
}

async function pair(bucket, env, request, now) {
  if (!(await setupMatches(env, request.headers.get("x-commy-setup")))) return reply(403, "wrong setup code");
  const signed = await readSigned(request);
  if (!signed || !canManage(signed.roster, signed.key)) return reply(400, "not a signed member list");
  if (signed.roster.at > now + SIGNED_FOR_MS) return reply(400, "the member list is from the future");
  await keepRoster(bucket, signed.roster, now);
  return reply(204);
}

async function updateRoster(bucket, request, now) {
  const current = await loadRoster(bucket, now);
  if (!current) return reply(409, "not set up yet");
  const signed = await readSigned(request);
  if (!signed) return reply(400, "not a signed member list");
  const next = signed.roster;
  if (next.at <= current.at || next.at > now + SIGNED_FOR_MS) return reply(409, "an older member list");
  if (next.owner !== current.owner) return reply(403, "only the setup code can change the owner");
  const byOwner = signed.key === current.owner;
  if (!byOwner && !(current.managers.includes(signed.key) && sameKeys(next.managers, current.managers))) {
    return reply(403, "only the owner can change who manages storage");
  }
  await keepRoster(bucket, next, now);
  return reply(204);
}

async function sendRoster(bucket, request, now) {
  const roster = await loadRoster(bucket, now);
  if (!roster) return reply(409, "not set up yet");
  const key = await signer(request, "GET", "roster", 0, now);
  if (!key || !canManage(roster, key)) return reply(403);
  return json(200, roster);
}

export function dailyCap(env) {
  const mb = Number(env.DAILY_UPLOAD_MB);
  return (Number.isFinite(mb) && mb > 0 ? mb : DEFAULT_DAILY_UPLOAD_MB) * MIB;
}

function dayOf(now) {
  return new Date(now).toISOString().slice(0, 10);
}

async function countUpload(bucket, env, key, size, now) {
  const day = dayOf(now);
  const object = await bucket.get(usageName(key));
  let used = 0;
  if (object) {
    try {
      const usage = JSON.parse(await object.text());
      if (usage.day === day && Number.isFinite(usage.bytes)) used = usage.bytes;
    } catch {
      used = 0;
    }
  }
  if (used + size > dailyCap(env)) return false;
  await bucket.put(usageName(key), JSON.stringify({ day, bytes: used + size }));
  return true;
}

async function check(bucket, id) {
  const head = await bucket.head(objectName(id));
  return head ? reply(200, null, { "content-length": String(head.size), "cache-control": "no-store" }) : reply(404, null, { "cache-control": "no-store" });
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

async function keepFile(bucket, env, id, request, key, now) {
  const declared = Number(request.headers.get("content-length"));
  if (!fitsSealed(declared) || !request.body) return reply(400, "not a sealed file");
  if (await bucket.head(objectName(id))) return reply(204);
  if (!(await countUpload(bucket, env, key, declared, now))) return reply(429, "daily upload limit reached");
  const kept = await bucket.put(objectName(id), request.body, { httpMetadata: { contentType: "application/octet-stream" } });
  if (!kept || kept.size !== declared) {
    await bucket.delete(objectName(id));
    return reply(400, "not a sealed file");
  }
  return reply(204);
}

async function file(bucket, env, request, method, id, now) {
  const roster = await loadRoster(bucket, now);
  if (!roster) return reply(403, "not set up yet");
  const length = method === "PUT" ? Number(request.headers.get("content-length")) || 0 : 0;
  const key = await signer(request, method, id, length, now);
  if (!key || !admits(roster, key)) return reply(403);
  if (method === "HEAD") return check(bucket, id);
  if (method === "GET") return fetchFile(bucket, id);
  return keepFile(bucket, env, id, request, key, now);
}

export async function handle(request, env, now = Date.now()) {
  const bucket = env.FILES;
  if (!bucket) return reply(503, "this worker has no FILES bucket bound");
  const path = new URL(request.url).pathname.replace(/\/+$/, "") || "/";
  const method = request.method;
  if (method === "OPTIONS") return reply(204);
  if (path === "/") {
    if (method !== "GET" && method !== "HEAD") return reply(405);
    const paired = Boolean(await loadRoster(bucket, now));
    return json(200, { commy_files: 2, max_bytes: MAX_FILE_PLAIN, paired });
  }
  if (path === "/_commy/pair") return method === "POST" ? pair(bucket, env, request, now) : reply(405);
  if (path === "/_commy/roster") {
    if (method === "GET") return sendRoster(bucket, request, now);
    if (method === "PUT") return updateRoster(bucket, request, now);
    return reply(405);
  }
  const id = FILE_ID.exec(path)?.[1];
  if (!id) return reply(404);
  if (method === "HEAD" || method === "GET" || method === "PUT") return file(bucket, env, request, method, id, now);
  return reply(405);
}

export default { fetch: (request, env) => handle(request, env) };
