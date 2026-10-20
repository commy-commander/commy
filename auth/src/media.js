import { createHmac } from "node:crypto";
import * as z from "zod";
import { APIError, createAuthEndpoint, getSessionFromCtx } from "better-auth/api";

const KLIPY_API = "https://api.klipy.com/api/v1";
const EMOJIGG_API = "https://emoji.gg/api/";
const EMOJIGG_TTL_MS = 6 * 60 * 60 * 1000;
const EMOJIGG_RETRY_MS = 5 * 60 * 1000;
const EMOJIGG_TIMEOUT_MS = 30_000;
const EMOJIGG_PAGE = 60;
const EMOJIGG_MAX_PAGE = 20;
const EMOJIGG_CATEGORIES = [
  [22, "Reactions"],
  [15, "Cute"],
  [3, "Meme"],
  [10, "Gaming"],
  [8, "Animated"],
  [17, "Animals"],
  [4, "Anime"],
  [6, "Blobs"],
  [13, "Pepe"],
  [7, "Thinking"],
  [20, "Hearts"],
  [21, "Pixel Art"],
  [23, "Aesthetic"],
  [2, "TV / Movie"],
  [1, "Original Style"],
  [18, "Recolors"],
  [11, "Letters"],
  [16, "Utility"],
  [19, "Flags"],
  [14, "Logos"],
  [5, "Celebrity"],
  [12, "Other"],
];
const EMOJIGG_HIDDEN_CATEGORY = 9;
const EMOJIGG_HIDDEN_WORDS = /(nsfw|porn|hentai|nude|naked|boob|titty|titties|penis|dick|cock|pussy|vagina|sex|cum|horny|lewd|nipple|thong|swastika|nazi|hitler|kkk|nigg|fag|retard)/i;
const TWITCH_TOKEN_URL = "https://id.twitch.tv/oauth2/token";
const IGDB_GAMES_URL = "https://api.igdb.com/v4/games";
const CACHE_TTL_MS = 10 * 60 * 1000;
const CACHE_CAP = 500;
const IGDB_MAX_REQUESTS_PER_SECOND = 4;
const IGDB_GAP_MARGIN_MS = 10;
const IGDB_GAP_MS = 1000 / IGDB_MAX_REQUESTS_PER_SECOND + IGDB_GAP_MARGIN_MS;
const TOKEN_REFRESH_EARLY_SECS = 60;
const MAX_QUERY = 100;
const MAX_STICKER_BYTES = 512 * 1024;
const STICKER_FORMATS = ["webp", "gif"];
const MAX_IMAGE_BYTES = 15 * 1024 * 1024;
const IMAGE_CACHE_BYTES = 64 * 1024 * 1024;
const IMAGE_CACHE_TTL_MS = 24 * 60 * 60 * 1000;
const IMAGE_TIMEOUT_MS = 15_000;

const MEDIA_HOSTS = [
  { host: "static.klipy.com", path: /^\/[A-Za-z0-9._~\-\/]{1,400}$/, query: 200 },
  { host: "images.igdb.com", path: /^\/igdb\/image\/upload\/t_[a-z0-9_]{1,40}\/[A-Za-z0-9]{1,32}\.(jpg|png|webp)$/, query: 0 },
  { host: /^(?:media[0-9]?|i)\.giphy\.com$/, path: /^\/(?:media\/[A-Za-z0-9._=\-]{1,500}\/)?[A-Za-z0-9_\-]{1,80}\.(?:gif|webp)$/, query: 300 },
  { host: "cdn3.emoji.gg", path: /^\/emojis\/[A-Za-z0-9_\-]{1,160}\.(?:png|gif)$/, query: 0 },
];

export function mediaHost(hostname) {
  return MEDIA_HOSTS.find((rule) => (typeof rule.host === "string" ? rule.host === hostname : rule.host.test(hostname))) ?? null;
}

export function mediaUrl(value) {
  let url;
  try {
    url = new URL(String(value ?? ""));
  } catch {
    return null;
  }
  const rule = mediaHost(url.hostname);
  if (url.protocol !== "https:" || url.username || url.password || url.port || url.hash) return null;
  if (!rule || !rule.path.test(url.pathname) || url.pathname.includes("..")) return null;
  if (url.search && url.search.length > rule.query) return null;
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

function stickerFile(file, sizes, fits) {
  for (const size of sizes) {
    for (const format of STICKER_FORMATS) {
      const r = file?.[size]?.[format];
      if (httpsUrl(r?.url) && fits(r)) return r;
    }
  }
  return null;
}

function stickerFits(r) {
  const size = Number(r?.size);
  return r?.size == null || (size > 0 && size <= MAX_STICKER_BYTES);
}

export function klipyStickerResults(body) {
  const items = Array.isArray(body?.data) ? body.data : Array.isArray(body?.data?.data) ? body.data.data : [];
  return items
    .map((item) => {
      const preview = stickerFile(item?.file, ["sm", "xs", "md"], () => true);
      const send = stickerFile(item?.file, ["md", "sm", "xs"], stickerFits);
      const ratio = preview?.width && preview?.height ? Number(preview.height) / Number(preview.width) : 0;
      return {
        id: String(item?.slug ?? item?.id ?? "").slice(0, 120),
        preview: preview?.url ?? null,
        url: send?.url ?? null,
        ratio: Number.isFinite(ratio) ? ratio : 0,
        title: String(item?.title || "Sticker").slice(0, 200),
      };
    })
    .filter((s) => s.id && s.preview && s.url);
}

export function emojiggCatalog(body) {
  if (!Array.isArray(body)) return [];
  return body
    .map((item) => {
      const url = mediaUrl(item?.image);
      const title = String(item?.title ?? "").trim().slice(0, 64);
      return {
        id: Number(item?.id),
        title,
        url: url?.hostname === "cdn3.emoji.gg" ? url.href : null,
        animated: Boolean(url?.pathname.endsWith(".gif")),
        category: Number(item?.category),
        faves: Number(item?.faves) || 0,
      };
    })
    .filter((e) => Number.isInteger(e.id) && e.title && e.url)
    .filter((e) => e.category !== EMOJIGG_HIDDEN_CATEGORY && !EMOJIGG_HIDDEN_WORDS.test(e.title))
    .sort((a, b) => b.faves - a.faves || b.id - a.id);
}

function searchWords(text) {
  return String(text)
    .toLowerCase()
    .replace(/^[0-9]+[_-]/, "")
    .split(/[^a-z0-9]+/)
    .filter(Boolean);
}

export function emojiggSearch(catalog, { q = "", category = null, page = 0 } = {}) {
  const words = searchWords(q);
  const start = Math.max(0, Math.min(EMOJIGG_MAX_PAGE, Math.trunc(Number(page) || 0))) * EMOJIGG_PAGE;
  const matches = catalog.filter((e) => {
    if (category && e.category !== category) return false;
    if (!words.length) return true;
    const title = e.title.toLowerCase();
    return words.every((w) => title.includes(w));
  });
  return {
    results: matches.slice(start, start + EMOJIGG_PAGE).map(({ id, title, url, animated }) => ({ id, title, url, animated })),
    more: matches.length > start + EMOJIGG_PAGE,
  };
}

export function emojiggCategory(value) {
  const id = Number(value);
  return EMOJIGG_CATEGORIES.some(([known]) => known === id) ? id : null;
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
  const stickerCache = createCache();
  const gameCache = createCache();
  const imageCache = createImageCache();
  let igdbToken = null;
  let igdbQueue = Promise.resolve();
  let emojigg = { items: [], fresh: 0, loading: null };

  async function loadEmojigg() {
    const res = await doFetch(EMOJIGG_API, { redirect: "error", signal: AbortSignal.timeout(EMOJIGG_TIMEOUT_MS) });
    if (!res.ok) throw new Error(`emoji.gg answered ${res.status}`);
    const items = emojiggCatalog(await res.json());
    if (!items.length) throw new Error("emoji.gg sent no emojis");
    return items;
  }

  async function emojiggItems() {
    if (emojigg.fresh > Date.now()) return emojigg.items;
    emojigg.loading ??= loadEmojigg()
      .then((items) => {
        emojigg = { items, fresh: Date.now() + EMOJIGG_TTL_MS, loading: null };
      })
      .catch((err) => {
        console.warn(`[auth] emoji.gg catalog failed: ${err.message}`);
        emojigg = { ...emojigg, fresh: emojigg.items.length ? Date.now() + EMOJIGG_RETRY_MS : 0, loading: null };
      });
    await emojigg.loading;
    if (!emojigg.items.length) throw new Error("no catalog yet");
    return emojigg.items;
  }

  async function twitchToken() {
    if (igdbToken && igdbToken.expires > Date.now()) return igdbToken.value;
    const url = new URL(TWITCH_TOKEN_URL);
    url.searchParams.set("client_id", igdb.clientId);
    url.searchParams.set("client_secret", igdb.clientSecret);
    url.searchParams.set("grant_type", "client_credentials");
    const res = await doFetch(url, { method: "POST" });
    if (!res.ok) throw new Error(`Twitch token request answered ${res.status}`);
    const body = await res.json();
    igdbToken = { value: body.access_token, expires: Date.now() + (Number(body.expires_in) - TOKEN_REFRESH_EARLY_SECS) * 1000 };
    return igdbToken.value;
  }

  function maskKlipyKey(message) {
    return String(message).replaceAll(klipyKey, "***");
  }

  const KLIPY_KINDS = {
    gifs: { cache: gifCache, what: "GIF search", service: "The GIF service", formats: "gif", results: klipyResults },
    stickers: { cache: stickerCache, what: "Sticker search", service: "The sticker service", formats: "webp,gif", results: klipyStickerResults },
  };

  async function klipySearch(ctx, kind) {
    const spec = KLIPY_KINDS[kind];
    if (!klipyKey) throw unavailable(spec.what);
    const session = await requireSession(ctx);
    const q = cleanQuery(ctx.query?.q);
    const key = q.toLowerCase();
    const cached = spec.cache.get(key);
    if (cached) return ctx.json({ results: cached, provider: "klipy" });

    const url = new URL(`${KLIPY_API}/${encodeURIComponent(klipyKey)}/${kind}/${q ? "search" : "trending"}`);
    if (q) url.searchParams.set("q", q);
    url.searchParams.set("per_page", "24");
    url.searchParams.set("page", "1");
    url.searchParams.set("content_filter", "medium");
    url.searchParams.set("format_filter", spec.formats);
    url.searchParams.set("customer_id", customerId(secret, session.user.id));
    let body;
    try {
      const res = await doFetch(url);
      body = await res.json().catch(() => ({}));
      if (!res.ok || body.result === false) throw new Error(`KLIPY answered ${res.status}`);
    } catch (err) {
      console.warn(`[auth] ${spec.what} failed: ${maskKlipyKey(err.message)}`);
      throw new APIError("BAD_GATEWAY", { message: `${spec.service} is not answering right now` });
    }
    const results = spec.results(body);
    spec.cache.set(key, results);
    return ctx.json({ results, provider: "klipy" });
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
          if (!url) throw new APIError("BAD_REQUEST", { message: "Only GIFs, emojis and game covers load through this server" });
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
        async (ctx) => klipySearch(ctx, "gifs"),
      ),

      searchCommyStickers: createAuthEndpoint(
        "/commy/stickers/search",
        { method: "GET", query: z.object({ q: z.string().optional() }) },
        async (ctx) => klipySearch(ctx, "stickers"),
      ),

      searchCommyEmojis: createAuthEndpoint(
        "/commy/emojis/search",
        {
          method: "GET",
          query: z.object({ q: z.string().optional(), category: z.string().optional(), page: z.string().optional() }),
        },
        async (ctx) => {
          await requireSession(ctx);
          let items;
          try {
            items = await emojiggItems();
          } catch {
            throw new APIError("BAD_GATEWAY", { message: "emoji.gg is not answering right now" });
          }
          const found = emojiggSearch(items, {
            q: cleanQuery(ctx.query?.q),
            category: emojiggCategory(ctx.query?.category),
            page: ctx.query?.page,
          });
          return ctx.json({
            ...found,
            categories: EMOJIGG_CATEGORIES.map(([id, name]) => ({ id, name })),
            provider: "emoji.gg",
          });
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
