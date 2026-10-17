import { createHmac } from "node:crypto";
import * as z from "zod";
import { APIError, createAuthEndpoint, getSessionFromCtx } from "better-auth/api";

const KLIPY_API = "https://api.klipy.com/api/v1";
const TWITCH_TOKEN_URL = "https://id.twitch.tv/oauth2/token";
const IGDB_GAMES_URL = "https://api.igdb.com/v4/games";
const CACHE_TTL_MS = 10 * 60 * 1000;
const CACHE_CAP = 500;
const IGDB_GAP_MS = 260;
const MAX_QUERY = 100;
const MAX_IMAGE_BYTES = 15 * 1024 * 1024;
const IMAGE_CACHE_BYTES = 64 * 1024 * 1024;
const IMAGE_CACHE_TTL_MS = 24 * 60 * 60 * 1000;
const IMAGE_TIMEOUT_MS = 15_000;

const MEDIA_HOSTS = {
  "static.klipy.com": /^\/[A-Za-z0-9._~\-\/]{1,400}$/,
  "images.igdb.com": /^\/igdb\/image\/upload\/t_[a-z0-9_]{1,40}\/[A-Za-z0-9]{1,32}\.(jpg|png|webp)$/,
};

export function mediaUrl(value) {
  let url;
  try {
    url = new URL(String(value ?? ""));
  } catch {
    return null;
  }
  const path = MEDIA_HOSTS[url.hostname];
  if (url.protocol !== "https:" || url.username || url.password || url.port || url.hash) return null;
  if (!path || !path.test(url.pathname) || url.pathname.includes("..")) return null;
  if (url.search && (url.hostname !== "static.klipy.com" || url.search.length > 200)) return null;
  return url;
}

export function cleanQuery(value) {
  return String(value ?? "")
    .replace(/[\u0000-\u001f\u007f]/g, "")
    .trim()
    .slice(0, MAX_QUERY);
}

export function customerId(secret, userId) {
  return createHmac("sha256", secret).update(`klipy:${userId}`).digest("hex").slice(0, 32);
}

function rendition(file, sizes) {
  for (const size of sizes) {
    const r = file?.[size]?.gif;
    if (r?.url) return r;
  }
  return null;
}

function httpsUrl(value) {
  return typeof value === "string" && value.startsWith("https://") ? value : null;
}

export function klipyResults(body) {
  const items = Array.isArray(body?.data) ? body.data : Array.isArray(body?.data?.data) ? body.data.data : [];
  return items
    .map((item) => {
      const preview = rendition(item?.file, ["sm", "xs", "md"]);
      const full = rendition(item?.file, ["md", "sm", "hd"]);
      const ratio = preview?.width && preview?.height ? Number(preview.height) / Number(preview.width) : 0;
      return {
        preview: httpsUrl(preview?.url),
        url: httpsUrl(full?.url),
        ratio: Number.isFinite(ratio) ? ratio : 0,
        title: String(item?.title || "GIF").slice(0, 200),
      };
    })
    .filter((g) => g.preview && g.url);
}

const IMAGE_ID = /^[A-Za-z0-9]{1,32}$/;

export function igdbResults(games) {
  if (!Array.isArray(games)) return [];
  return games
    .map((game) => {
      const name = String(game?.name ?? "").trim();
      const cover = game?.cover?.image_id;
      const year = Number.isFinite(game?.first_release_date)
        ? new Date(game.first_release_date * 1000).getUTCFullYear()
        : null;
      return { id: Number(game?.id), name, cover: IMAGE_ID.test(cover ?? "") ? cover : null, year };
    })
    .filter((g) => Number.isInteger(g.id) && g.name);
}

export function apicalypseString(query) {
  return query.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

function createCache() {
  const entries = new Map();
  return {
    get(key) {
      const hit = entries.get(key);
      if (!hit || hit.expires < Date.now()) return undefined;
      return hit.value;
    },
    set(key, value) {
      if (entries.size >= CACHE_CAP) entries.delete(entries.keys().next().value);
      entries.set(key, { value, expires: Date.now() + CACHE_TTL_MS });
    },
  };
}

function createImageCache() {
  const entries = new Map();
  let bytes = 0;
  const drop = (key) => {
    bytes -= entries.get(key).body.byteLength;
    entries.delete(key);
  };
  return {
    get(key) {
      const hit = entries.get(key);
      if (!hit) return undefined;
      if (hit.expires < Date.now()) {
        drop(key);
        return undefined;
      }
      return hit;
    },
    set(key, type, body) {
      if (body.byteLength > IMAGE_CACHE_BYTES / 4) return;
      if (entries.has(key)) drop(key);
      while (bytes + body.byteLength > IMAGE_CACHE_BYTES) drop(entries.keys().next().value);
      entries.set(key, { type, body, expires: Date.now() + IMAGE_CACHE_TTL_MS });
      bytes += body.byteLength;
    },
  };
}

function imageResponse(type, body) {
  return new Response(body, {
    headers: {
      "content-type": type,
      "content-length": String(body.byteLength),
      "cache-control": "private, max-age=86400",
      "x-content-type-options": "nosniff",
      "content-security-policy": "default-src 'none'",
    },
  });
}

function unavailable(what) {
  return new APIError("NOT_FOUND", { message: `${what} is not set up on this account server` });
}

async function requireSession(ctx) {
  const session = await getSessionFromCtx(ctx).catch(() => null);
  if (!session) throw new APIError("UNAUTHORIZED", { message: "Sign in to search" });
  return session;
}

export const commyMedia = ({ secret, klipyKey = null, igdb = null, fetch: doFetch = (...args) => globalThis.fetch(...args) }) => {
  const gifCache = createCache();
  const gameCache = createCache();
  const imageCache = createImageCache();
  let igdbToken = null;
  let igdbQueue = Promise.resolve();

  async function twitchToken() {
    if (igdbToken && igdbToken.expires > Date.now()) return igdbToken.value;
    const url = new URL(TWITCH_TOKEN_URL);
    url.searchParams.set("client_id", igdb.clientId);
    url.searchParams.set("client_secret", igdb.clientSecret);
    url.searchParams.set("grant_type", "client_credentials");
    const res = await doFetch(url, { method: "POST" });
    if (!res.ok) throw new Error(`Twitch token request answered ${res.status}`);
    const body = await res.json();
    igdbToken = { value: body.access_token, expires: Date.now() + (Number(body.expires_in) - 60) * 1000 };
    return igdbToken.value;
  }

  function paced(task) {
    const run = igdbQueue.then(task);
    igdbQueue = run.catch(() => {}).then(() => new Promise((r) => setTimeout(r, IGDB_GAP_MS)));
    return run;
  }

  async function searchIgdb(query, retry = true) {
    const token = await twitchToken();
    const res = await paced(() =>
      doFetch(IGDB_GAMES_URL, {
        method: "POST",
        headers: { "Client-ID": igdb.clientId, Authorization: `Bearer ${token}` },
        body: `search "${apicalypseString(query)}"; fields name,cover.image_id,first_release_date; where version_parent = null; limit 8;`,
      }),
    );
    if (res.status === 401 && retry) {
      igdbToken = null;
      return searchIgdb(query, false);
    }
    if (!res.ok) throw new Error(`IGDB answered ${res.status}`);
    return igdbResults(await res.json());
  }

  async function fetchImage(url) {
    const res = await doFetch(url, { redirect: "error", signal: AbortSignal.timeout(IMAGE_TIMEOUT_MS) });
    if (!res.ok) throw new Error(`answered ${res.status}`);
    const type = (res.headers.get("content-type") ?? "").split(";")[0].trim().toLowerCase();
    if (!/^image\/(gif|jpeg|png|webp)$/.test(type)) throw new Error(`sent ${type || "no type"}`);
    if (Number(res.headers.get("content-length") ?? 0) > MAX_IMAGE_BYTES) throw new Error("image too large");
    const body = new Uint8Array(await res.arrayBuffer());
    if (body.byteLength > MAX_IMAGE_BYTES) throw new Error("image too large");
    return { type, body };
  }

  return {
    id: "commy-media",
    endpoints: {
      commyMediaImage: createAuthEndpoint(
        "/commy/media",
        { method: "GET", query: z.object({ u: z.string() }) },
        async (ctx) => {
          await requireSession(ctx);
          const url = mediaUrl(ctx.query.u);
          if (!url) throw new APIError("BAD_REQUEST", { message: "Only GIFs and game covers load through this server" });
          const cached = imageCache.get(url.href);
          if (cached) return imageResponse(cached.type, cached.body);
          let image;
          try {
            image = await fetchImage(url);
          } catch (err) {
            console.warn(`[auth] image from ${url.hostname} failed: ${err.message}`);
            throw new APIError("BAD_GATEWAY", { message: "The image could not be loaded right now" });
          }
          imageCache.set(url.href, image.type, image.body);
          return imageResponse(image.type, image.body);
        },
      ),

      searchCommyGifs: createAuthEndpoint(
        "/commy/gifs/search",
        { method: "GET", query: z.object({ q: z.string().optional() }) },
        async (ctx) => {
          if (!klipyKey) throw unavailable("GIF search");
          const session = await requireSession(ctx);
          const q = cleanQuery(ctx.query?.q);
          const key = q.toLowerCase();
          const cached = gifCache.get(key);
          if (cached) return ctx.json({ results: cached, provider: "klipy" });

          const url = new URL(`${KLIPY_API}/${encodeURIComponent(klipyKey)}/gifs/${q ? "search" : "trending"}`);
          if (q) url.searchParams.set("q", q);
          url.searchParams.set("per_page", "24");
          url.searchParams.set("page", "1");
          url.searchParams.set("content_filter", "medium");
          url.searchParams.set("format_filter", "gif");
          url.searchParams.set("customer_id", customerId(secret, session.user.id));
          let body;
          try {
            const res = await doFetch(url);
            body = await res.json().catch(() => ({}));
            if (!res.ok || body.result === false) throw new Error(`KLIPY answered ${res.status}`);
          } catch (err) {
            console.warn(`[auth] GIF search failed: ${String(err.message).replaceAll(klipyKey, "***")}`);
            throw new APIError("BAD_GATEWAY", { message: "The GIF service is not answering right now" });
          }
          const results = klipyResults(body);
          gifCache.set(key, results);
          return ctx.json({ results, provider: "klipy" });
        },
      ),

      searchCommyGames: createAuthEndpoint(
        "/commy/games/search",
        { method: "GET", query: z.object({ q: z.string() }) },
        async (ctx) => {
          if (!igdb) throw unavailable("Game search");
          await requireSession(ctx);
          const q = cleanQuery(ctx.query.q);
          if (q.length < 2) return ctx.json({ results: [] });
          const key = q.toLowerCase();
          const cached = gameCache.get(key);
          if (cached) return ctx.json({ results: cached });
          let results;
          try {
            results = await searchIgdb(q);
          } catch (err) {
            console.warn(`[auth] game search failed: ${err.message}`);
            throw new APIError("BAD_GATEWAY", { message: "The game database is not answering right now" });
          }
          gameCache.set(key, results);
          return ctx.json({ results });
        },
      ),
    },
  };
};
