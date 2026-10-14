// GIF search (KLIPY) and game lookups (IGDB) for signed in apps, using keys
// held by this server. Apps never see the keys, so a release build carries
// nothing worth extracting, and the operator can rotate a key without a new
// app release. Either half is off when its key is not configured.
import { createHmac } from "node:crypto";
import * as z from "zod";
import { APIError, createAuthEndpoint, getSessionFromCtx } from "better-auth/api";

const KLIPY_API = "https://api.klipy.com/api/v1";
const TWITCH_TOKEN_URL = "https://id.twitch.tv/oauth2/token";
const IGDB_GAMES_URL = "https://api.igdb.com/v4/games";
const CACHE_TTL_MS = 10 * 60 * 1000;
const CACHE_CAP = 500;
// IGDB allows 4 requests per second per client
const IGDB_GAP_MS = 260;
const MAX_QUERY = 100;

export function cleanQuery(value) {
  return String(value ?? "")
    .replace(/[\u0000-\u001f\u007f]/g, "")
    .trim()
    .slice(0, MAX_QUERY);
}

// KLIPY uses this to personalise results; a keyed hash keeps account ids private.
export function customerId(secret, userId) {
  return createHmac("sha256", secret).update(`klipy:${userId}`).digest("hex").slice(0, 32);
}

// KLIPY nests renditions as file.<size>.gif.{url,width,height} (sizes xs, sm, md, hd).
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
      // md is a good chat size; hd can be over 10 MB
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

// Escapes a query for IGDB's Apicalypse `search "..."` clause.
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
    // Refresh a minute early
    igdbToken = { value: body.access_token, expires: Date.now() + (Number(body.expires_in) - 60) * 1000 };
    return igdbToken.value;
  }

  // One IGDB request at a time, spaced to stay under its rate limit
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

  return {
    id: "commy-media",
    endpoints: {
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
            // The message can include the request URL, which holds the key
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
