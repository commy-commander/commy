import { createReadStream, readdirSync, readFileSync, statSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, extname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { siteLinks } from "./pages.js";

export const WEB_CLIENT_DIR = join(dirname(fileURLToPath(import.meta.url)), "..", "web");
export const WEB_PATH = "/web";
const RELAYS_TAG = /<meta name="commy-relays" content="[^"]*" \/>/;

const SECURITY_HEADERS = {
  "x-frame-options": "DENY",
  "content-security-policy": "frame-ancestors 'none'",
  "x-content-type-options": "nosniff",
  "referrer-policy": "strict-origin-when-cross-origin",
};

const CONTENT_TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json",
  ".wasm": "application/wasm",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".woff2": "font/woff2",
  ".mp3": "audio/mpeg",
  ".wav": "audio/wav",
  ".md": "text/plain; charset=utf-8",
  ".txt": "text/plain; charset=utf-8",
  "": "text/plain; charset=utf-8",
};
const BINARY = "application/octet-stream";

export function configuredRelays(env = process.env) {
  const relays = String(env.COMMY_RELAY_URLS ?? "")
    .split(",")
    .map((relay) => relay.trim().replace(/\/+$/, ""))
    .filter((relay) => /^https?:\/\/[^\s"<>]+$/.test(relay));
  return relays.length ? relays.join(",") : null;
}

export function pageWithRelays(html, relays) {
  if (!relays) return html;
  return html.replace(RELAYS_TAG, () => `<meta name="commy-relays" content="${relays}" />`);
}

export function isPhone(userAgent) {
  const ua = String(userAgent ?? "").toLowerCase();
  return /iphone|ipad|ipod|android/.test(ua);
}

export function withQuery(link, name, value) {
  const [path, hash = ""] = link.split("#");
  const joined = `${path}${path.includes("?") ? "&" : "?"}${name}=${encodeURIComponent(value)}`;
  return hash ? `${joined}#${hash}` : joined;
}

function listFiles(dir) {
  const files = new Map();
  const walk = (folder) => {
    for (const entry of readdirSync(folder, { withFileTypes: true })) {
      const full = join(folder, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.isFile()) {
        const stat = statSync(full);
        const name = relative(dir, full).split(sep).join("/");
        files.set(name, {
          full,
          size: stat.size,
          type: CONTENT_TYPES[extname(name)] ?? BINARY,
          etag: `W/"${stat.size.toString(16)}-${Math.floor(stat.mtimeMs).toString(16)}"`,
        });
      }
    }
  };
  walk(dir);
  return files;
}

function fresh(req, etag) {
  const asked = req.headers["if-none-match"];
  return Boolean(asked) && asked.split(",").some((tag) => tag.trim() === etag);
}

export function createWebClient({ env = process.env, dir = WEB_CLIENT_DIR } = {}) {
  let files;
  let index;
  try {
    files = listFiles(dir);
    index = readFileSync(join(dir, "index.html"), "utf8");
  } catch (err) {
    return { enabled: false, reason: `the web app is off because ${dir} has no build (${err.code ?? err.message}); build it with COMMY_WEB_OUT=${dir} COMMY_WEB_BASE=/web/ node apps/web/build.mjs` };
  }
  const page = pageWithRelays(index, configuredRelays(env));
  const pageTag = `"${createHash("sha256").update(page).digest("hex").slice(0, 32)}"`;
  const phoneTo = withQuery(siteLinks(env).download, "from", "web");

  function serveIndex(req, res) {
    if (isPhone(req.headers["user-agent"])) {
      res.writeHead(307, { location: phoneTo, "cache-control": "no-store", vary: "User-Agent" });
      return res.end();
    }
    const headers = { ...SECURITY_HEADERS, "content-type": CONTENT_TYPES[".html"], "cache-control": "no-cache", vary: "User-Agent", etag: pageTag };
    if (fresh(req, pageTag)) {
      res.writeHead(304, headers);
      return res.end();
    }
    res.writeHead(200, { ...headers, "content-length": Buffer.byteLength(page) });
    res.end(req.method === "HEAD" ? undefined : page);
  }

  function serveFile(req, res, file) {
    const headers = { ...SECURITY_HEADERS, "content-type": file.type, "cache-control": "public, max-age=0, must-revalidate", etag: file.etag };
    if (fresh(req, file.etag)) {
      res.writeHead(304, headers);
      return res.end();
    }
    res.writeHead(200, { ...headers, "content-length": file.size });
    if (req.method === "HEAD") return res.end();
    createReadStream(file.full).on("error", () => res.destroy()).pipe(res);
  }

  return {
    enabled: true,
    handles(req) {
      if (req.method !== "GET" && req.method !== "HEAD") return false;
      const path = (req.url ?? "/").split("?")[0];
      return path === WEB_PATH || path.startsWith(`${WEB_PATH}/`);
    },
    respond(req, res) {
      const path = (req.url ?? "/").split("?")[0];
      let name;
      try {
        name = decodeURIComponent(path.slice(WEB_PATH.length + 1));
      } catch {
        name = null;
      }
      if (path === WEB_PATH || name === "" || name === "index.html") return serveIndex(req, res);
      const file = name === null ? null : files.get(name);
      if (!file) {
        res.writeHead(404, { "content-type": "application/json", "cache-control": "no-store", "x-content-type-options": "nosniff" });
        return res.end(req.method === "HEAD" ? undefined : '{"error":"not_found"}');
      }
      return serveFile(req, res, file);
    },
  };
}
