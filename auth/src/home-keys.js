import dns from "node:dns";
import https from "node:https";
import net from "node:net";
import * as z from "zod";
import { APIError, createAuthEndpoint, sessionMiddleware } from "better-auth/api";
import { keyId } from "./server-key.js";

export const HOME_KEY_PATH = "/commy/home-key";
export const DISCOVERY_PATH = "/.well-known/commy-auth.json";
const FOUND_FOR_MS = 60 * 60 * 1000;
export const MISSING_FOR_MS = 30 * 1000;
const FETCH_TIMEOUT_MS = 8000;
const MAX_DOCUMENT_BYTES = 64 * 1024;
const MAX_REMEMBERED = 5000;
const HOME_ORIGIN = /^https:\/\/(?=.{1,253}$)([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z][a-z0-9-]{0,61}[a-z0-9]$/;
const PRIVATE_ENDINGS = [".local", ".localhost", ".internal", ".lan", ".home", ".arpa", ".test", ".invalid", ".example"];

const UNREACHABLE_RANGES = [
  ["0.0.0.0", 8, "ipv4"],
  ["10.0.0.0", 8, "ipv4"],
  ["100.64.0.0", 10, "ipv4"],
  ["127.0.0.0", 8, "ipv4"],
  ["169.254.0.0", 16, "ipv4"],
  ["172.16.0.0", 12, "ipv4"],
  ["192.0.0.0", 24, "ipv4"],
  ["192.168.0.0", 16, "ipv4"],
  ["198.18.0.0", 15, "ipv4"],
  ["224.0.0.0", 3, "ipv4"],
  ["::", 128, "ipv6"],
  ["::1", 128, "ipv6"],
  ["64:ff9b::", 96, "ipv6"],
  ["fc00::", 7, "ipv6"],
  ["fe80::", 10, "ipv6"],
  ["ff00::", 8, "ipv6"],
];

const unreachable = new net.BlockList();
for (const [address, prefix, family] of UNREACHABLE_RANGES) unreachable.addSubnet(address, prefix, family);

const IPV4_MAPPED = /^::ffff:(\d+\.\d+\.\d+\.\d+)$/i;

export function isPublicAddress(address) {
  const plain = String(address ?? "").replace(IPV4_MAPPED, "$1");
  const family = net.isIP(plain);
  return family !== 0 && !unreachable.check(plain, `ipv${family}`);
}

export function isHomeOrigin(home) {
  const text = String(home ?? "");
  return HOME_ORIGIN.test(text) && !PRIVATE_ENDINGS.some((ending) => text.endsWith(ending));
}

export function publicOnlyLookup(hostname, options, callback) {
  dns.lookup(hostname, { ...options, all: true }, (err, addresses) => {
    if (err) return callback(err);
    const usable = addresses.filter((a) => isPublicAddress(a.address));
    if (usable.length === 0) return callback(Object.assign(new Error("no public address"), { code: "ENOTPUBLIC" }));
    if (options?.all) return callback(null, usable);
    callback(null, usable[0].address, usable[0].family);
  });
}

export function fetchDiscovery(home, { lookup = publicOnlyLookup } = {}) {
  return new Promise((resolve, reject) => {
    const req = https.get(`${home}${DISCOVERY_PATH}`, { lookup, timeout: FETCH_TIMEOUT_MS, headers: { accept: "application/json" } }, (res) => {
      if (res.statusCode !== 200) {
        res.resume();
        return reject(new Error(`answered ${res.statusCode}`));
      }
      let size = 0;
      const chunks = [];
      res.on("data", (chunk) => {
        size += chunk.length;
        if (size > MAX_DOCUMENT_BYTES) return req.destroy(new Error("too large"));
        chunks.push(chunk);
      });
      res.on("end", () => {
        try {
          resolve(JSON.parse(Buffer.concat(chunks).toString("utf8")));
        } catch {
          reject(new Error("not JSON"));
        }
      });
    });
    req.on("timeout", () => req.destroy(new Error("timed out")));
    req.on("error", reject);
  });
}

export function publishedKeyOf(document, home) {
  const key = document?.serverKey;
  const wellFormed = key?.alg === "ed25519" && /^[0-9a-f]{64}$/.test(key?.public ?? "") && key.id === keyId(key.public);
  if (!wellFormed || document.issuer !== home) return null;
  return { alg: key.alg, id: key.id, public: key.public };
}

export function syncUrlOf(document, home) {
  const offered = String(document?.endpoints?.sync ?? "").trim().replace(/\/+$/, "");
  if (document?.issuer !== home || !offered) return null;
  let url;
  try {
    url = new URL(offered);
  } catch {
    return null;
  }
  const sameHost = url.hostname === new URL(home).hostname;
  const plain = !url.username && !url.password && !url.search && !url.hash;
  return url.protocol === "wss:" && sameHost && plain ? offered : null;
}

export function createHomeKeys({ ownHome, ownKey, ownSync = null, fetchDocument = fetchDiscovery, now = () => Date.now() } = {}) {
  const remembered = new Map();

  function remember(home, found) {
    if (remembered.size >= MAX_REMEMBERED) remembered.delete(remembered.keys().next().value);
    const usable = Boolean(found.key || found.sync);
    remembered.set(home, { ...found, until: now() + (usable ? FOUND_FOR_MS : MISSING_FOR_MS) });
    return found;
  }

  async function homeOf(home) {
    if (home === ownHome) return { key: ownKey, sync: ownSync };
    const known = remembered.get(home);
    if (known && known.until > now()) return { key: known.key, sync: known.sync };
    const document = await fetchDocument(home).catch(() => null);
    return remember(home, { key: publishedKeyOf(document, home), sync: syncUrlOf(document, home) });
  }

  async function keyOf(home) {
    return (await homeOf(home)).key;
  }

  return { keyOf, homeOf };
}

export const commyHomeKeys = ({ homeKeys }) => ({
  id: "commy-home-keys",
  endpoints: {
    homeKey: createAuthEndpoint(
      HOME_KEY_PATH,
      { method: "GET", use: [sessionMiddleware], query: z.object({ home: z.string() }) },
      async (ctx) => {
        const home = ctx.query.home;
        if (!isHomeOrigin(home)) throw new APIError("BAD_REQUEST", { message: "home must be an https address on a public domain" });
        const { key, sync } = await homeKeys.homeOf(home);
        if (!key && !sync) throw new APIError("NOT_FOUND", { message: "that server does not publish a server key" });
        return ctx.json({ home, key, sync });
      },
    ),
  },
});
