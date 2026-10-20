import assert from "node:assert/strict";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, featureSummary, loadConfig } from "../src/config.js";
import {
  apicalypseString,
  cleanQuery,
  customerId,
  emojiggCatalog,
  emojiggCategory,
  emojiggSearch,
  igdbResults,
  klipyResults,
  klipyStickerResults,
  mediaHost,
  mediaUrl,
} from "../src/media.js";

const ORIGIN = "http://localhost:3001";
const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: ORIGIN };
const KLIPY_KEY = "klipy-test-key";

const klipyBody = {
  result: true,
  data: {
    data: [
      {
        title: "Wave",
        file: {
          sm: { gif: { url: "https://static.klipy.com/sm.gif", width: 200, height: 100 } },
          md: { gif: { url: "https://static.klipy.com/md.gif", width: 400, height: 200 } },
        },
      },
      { title: "No files", file: {} },
      { title: "Plain http", file: { sm: { gif: { url: "http://x/sm.gif" } }, md: { gif: { url: "http://x/md.gif" } } } },
    ],
  },
};

const stickerBody = {
  result: true,
  data: {
    data: [
      {
        slug: "grrr-rawr",
        title: "Grrr Rawr",
        file: {
          hd: { webp: { url: "https://static.klipy.com/hd.webp", width: 498, height: 498, size: 900000 } },
          md: {
            webp: { url: "https://static.klipy.com/md.webp", width: 300, height: 300, size: 700000 },
            gif: { url: "https://static.klipy.com/md.gif", width: 300, height: 300, size: 400000 },
          },
          sm: { webp: { url: "https://static.klipy.com/sm.webp", width: 150, height: 120, size: 90000 } },
        },
      },
      {
        slug: "too-big",
        title: "Too big",
        file: { sm: { gif: { url: "https://static.klipy.com/big.gif", width: 150, height: 150, size: 900000 } } },
      },
      { slug: "", title: "No slug", file: { sm: { webp: { url: "https://static.klipy.com/x.webp" } } } },
    ],
  },
};

function fakeFetch(log) {
  return async (url, init = {}) => {
    const u = String(url);
    log.push({ url: u, init });
    if (u.startsWith("https://api.klipy.com/")) return Response.json(u.includes("/stickers/") ? stickerBody : klipyBody);
    if (u === "https://emoji.gg/api/") return Response.json(emojiggBody);
    if (u.startsWith("https://id.twitch.tv/")) return Response.json({ access_token: "twitch-token", expires_in: 3600 });
    if (u.startsWith("https://api.igdb.com/")) {
      return Response.json([{ id: 72, name: "Portal 2", cover: { image_id: "co1rs4" }, first_release_date: 1303171200 }]);
    }
    if (u.startsWith("https://static.klipy.com/") && u.endsWith(".gif")) {
      return new Response(GIF_BYTES, { headers: { "content-type": "image/gif" } });
    }
    if (u.startsWith("https://static.klipy.com/")) return new Response("<html>", { headers: { "content-type": "text/html" } });
    if (u.startsWith("https://images.igdb.com/")) return new Response(GIF_BYTES, { headers: { "content-type": "image/jpeg" } });
    if (/^https:\/\/[a-z0-9]+\.giphy\.com\//.test(u)) return new Response(GIF_BYTES, { headers: { "content-type": "image/gif" } });
    return new Response("not found", { status: 404 });
  };
}

const GIF_BYTES = new Uint8Array([0x47, 0x49, 0x46, 0x38, 0x39, 0x61]);

const emojiggBody = [
  { id: 3, title: "party_blob", image: "https://cdn3.emoji.gg/emojis/3_party_blob.gif", category: 6, faves: 9 },
  { id: 2, title: "cat_wave", image: "https://cdn3.emoji.gg/emojis/2_cat_wave.png", category: 17, faves: 40 },
  { id: 4, title: "spicy", image: "https://cdn3.emoji.gg/emojis/4_spicy.png", category: 9, faves: 99 },
  { id: 5, title: "elsewhere", image: "https://evil.example/emojis/5.png", category: 17, faves: 1 },
  { id: 6, title: "", image: "https://cdn3.emoji.gg/emojis/6.png", category: 17 },
  { id: 7, title: "lewd_cat", image: "https://cdn3.emoji.gg/emojis/7_lewd_cat.png", category: 17 },
];

function makeAuth(extraEnv = {}) {
  const log = [];
  const options = buildAuthOptions(loadConfig({ ...ENV, ...extraEnv }), { pool: null, send: async () => {}, fetch: fakeFetch(log) });
  const db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], inviteBackground: [], doorbellRing: [] };
  options.database = memoryAdapter(db);
  options.rateLimit = { enabled: false };
  const auth = betterAuth(options);
  const call = async (method, path, { body, cookie, query } = {}) => {
    const url = new URL(`${ORIGIN}/api/auth${path}`);
    for (const [k, v] of Object.entries(query ?? {})) url.searchParams.set(k, v);
    const headers = { origin: ORIGIN };
    if (cookie) headers.cookie = cookie;
    if (body) headers["content-type"] = "application/json";
    const res = await auth.handler(new Request(url, { method, headers, body: body ? JSON.stringify(body) : undefined }));
    const type = res.headers.get("content-type") ?? "";
    if (type.startsWith("image/")) {
      return { status: res.status, type, bytes: new Uint8Array(await res.arrayBuffer()), headers: res.headers };
    }
    const text = await res.text();
    return { status: res.status, json: text ? JSON.parse(text) : null, setCookie: res.headers.get("set-cookie") };
  };
  const signUp = async (email) => {
    const r = await call("POST", "/sign-up/email", { body: { email, password: "correct horse battery", name: "Tester" } });
    assert.equal(r.status, 200, JSON.stringify(r.json));
    return r.setCookie.split(";")[0];
  };
  return { call, signUp, log };
}

test("KLIPY results keep https renditions only", () => {
  const results = klipyResults(klipyBody);
  assert.deepEqual(results, [
    { preview: "https://static.klipy.com/sm.gif", url: "https://static.klipy.com/md.gif", ratio: 0.5, title: "Wave" },
  ]);
  assert.deepEqual(klipyResults(null), []);
  assert.deepEqual(klipyResults({ data: "nope" }), []);
});

test("KLIPY stickers send the first rendition small enough to keep", () => {
  assert.deepEqual(klipyStickerResults(stickerBody), [
    { id: "grrr-rawr", preview: "https://static.klipy.com/sm.webp", url: "https://static.klipy.com/md.gif", ratio: 0.8, title: "Grrr Rawr" },
  ]);
  assert.deepEqual(klipyStickerResults(null), []);
});

test("the emoji.gg catalog keeps safe, named, CDN-hosted emojis, most loved first", () => {
  const catalog = emojiggCatalog(emojiggBody);
  assert.deepEqual(
    catalog.map((e) => [e.id, e.title, e.animated]),
    [
      [2, "cat_wave", false],
      [3, "party_blob", true],
    ],
  );
  assert.deepEqual(emojiggCatalog({ nope: true }), []);
});

test("emoji.gg search matches every word, filters by category and pages", () => {
  const catalog = emojiggCatalog(emojiggBody);
  assert.deepEqual(emojiggSearch(catalog, { q: "Cat Wave" }).results.map((e) => e.id), [2]);
  assert.deepEqual(emojiggSearch(catalog, { q: "cat party" }).results, []);
  assert.deepEqual(emojiggSearch(catalog, { category: 6 }).results.map((e) => e.id), [3]);
  assert.deepEqual(emojiggSearch(catalog, { page: 1 }), { results: [], more: false });
  const many = Array.from({ length: 70 }, (_, i) => ({ id: i, title: `e${i}`, url: "https://cdn3.emoji.gg/emojis/x.png", animated: false, category: 1 }));
  const first = emojiggSearch(many, {});
  assert.equal(first.results.length, 60);
  assert.equal(first.more, true);
  assert.equal(emojiggSearch(many, { page: "1" }).results.length, 10);
  assert.equal(emojiggCategory("6"), 6);
  assert.equal(emojiggCategory("9"), null, "the adult category cannot be asked for");
  assert.equal(emojiggCategory("x"), null);
});

test("emoji search needs a session and loads the catalog once", async () => {
  const { call, signUp, log } = makeAuth();
  assert.equal((await call("GET", "/commy/emojis/search", { query: { q: "cat" } })).status, 401);
  assert.equal(log.length, 0);
  const cookie = await signUp("emojis@example.test");
  const r = await call("GET", "/commy/emojis/search", { cookie, query: { q: "cat" } });
  assert.equal(r.status, 200, JSON.stringify(r.json));
  assert.deepEqual(r.json.results, [{ id: 2, title: "cat_wave", url: "https://cdn3.emoji.gg/emojis/2_cat_wave.png", animated: false }]);
  assert.ok(r.json.categories.some((c) => c.name === "Reactions"));
  assert.ok(!r.json.categories.some((c) => c.id === 9));
  await call("GET", "/commy/emojis/search", { cookie, query: { category: "6" } });
  assert.equal(log.filter((l) => l.url === "https://emoji.gg/api/").length, 1);
});

test("IGDB results drop unsafe cover ids and unnamed games", () => {
  const results = igdbResults([
    { id: 7, name: " Celeste ", cover: { image_id: "co3byy" }, first_release_date: 1516665600 },
    { id: 8, name: "Evil", cover: { image_id: "../x" } },
    { id: 9 },
  ]);
  assert.deepEqual(results, [
    { id: 7, name: "Celeste", cover: "co3byy", year: 2018 },
    { id: 8, name: "Evil", cover: null, year: null },
  ]);
  assert.deepEqual(igdbResults({}), []);
});

test("queries are cleaned and escaped", () => {
  assert.equal(cleanQuery("  cats\n\u0000 "), "cats");
  assert.equal(cleanQuery("x".repeat(500)).length, 100);
  assert.equal(apicalypseString('Tom "Clancy" \\ x'), 'Tom \\"Clancy\\" \\\\ x');
});

test("customer ids are stable and do not reveal the account id", () => {
  const a = customerId("secret", "user-1");
  assert.equal(a, customerId("secret", "user-1"));
  assert.notEqual(a, customerId("secret", "user-2"));
  assert.notEqual(a, customerId("other", "user-1"));
  assert.ok(!a.includes("user-1"));
});

test("discovery advertises only the searches with keys", () => {
  const none = discoveryDocument(loadConfig(ENV));
  assert.deepEqual(none.features, { gifs: false, stickers: false, games: false, emojis: true, media: true, vault: true, doorbell: true, twoFactor: true, changeEmail: false });
  const both = discoveryDocument(loadConfig({ ...ENV, KLIPY_API_KEY: KLIPY_KEY, TWITCH_CLIENT_ID: "id", TWITCH_CLIENT_SECRET: "s" }));
  assert.deepEqual(both.features, { gifs: true, stickers: true, games: true, emojis: true, media: true, vault: true, doorbell: true, twoFactor: true, changeEmail: false });
  assert.equal(both.endpoints.gifSearch, `${ORIGIN}/api/auth/commy/gifs/search`);
  assert.equal(both.endpoints.stickerSearch, `${ORIGIN}/api/auth/commy/stickers/search`);
  assert.equal(both.endpoints.gameSearch, `${ORIGIN}/api/auth/commy/games/search`);
  assert.ok(!JSON.stringify(both).includes(KLIPY_KEY), "the key never appears in discovery");
});

test("separate IGDB credentials win over the Twitch sign-in app", () => {
  const config = loadConfig({ ...ENV, TWITCH_CLIENT_ID: "tw", TWITCH_CLIENT_SECRET: "tws", IGDB_CLIENT_ID: "ig", IGDB_CLIENT_SECRET: "igs" });
  assert.deepEqual(config.igdb, { clientId: "ig", clientSecret: "igs" });
});

test("half a client pair is off with a warning, and pasted spaces are trimmed", () => {
  const config = loadConfig({ ...ENV, TWITCH_CLIENT_ID: " tw ", TWITCH_CLIENT_SECRET: "", IGDB_CLIENT_ID: " ig\n", IGDB_CLIENT_SECRET: " igs " });
  assert.equal(config.twitch, null);
  assert.deepEqual(config.igdb, { clientId: "ig", clientSecret: "igs" });
  assert.equal(config.warnings.length, 1);
  assert.match(config.warnings[0], /TWITCH_CLIENT_SECRET is empty/);
  assert.deepEqual(loadConfig(ENV).warnings, []);
});

test("the startup summary names each optional part without its keys", () => {
  const line = featureSummary(loadConfig({ ...ENV, KLIPY_API_KEY: KLIPY_KEY, TWITCH_CLIENT_ID: "tw-id", TWITCH_CLIENT_SECRET: "tw-secret" }));
  assert.match(line, /twitch=on/);
  assert.match(line, /gifs=on/);
  assert.match(line, /games=on \(twitch app\)/);
  assert.match(line, /google=off/);
  for (const secret of [KLIPY_KEY, "tw-id", "tw-secret"]) assert.ok(!line.includes(secret));
});

test("searches without keys say they are not set up", async () => {
  const { call, signUp } = makeAuth();
  const cookie = await signUp("nokeys@example.test");
  assert.equal((await call("GET", "/commy/gifs/search", { cookie, query: { q: "cat" } })).status, 404);
  assert.equal((await call("GET", "/commy/stickers/search", { cookie, query: { q: "cat" } })).status, 404);
  assert.equal((await call("GET", "/commy/games/search", { cookie, query: { q: "portal" } })).status, 404);
});

test("GIF search needs a session and keeps the key server side", async () => {
  const { call, signUp, log } = makeAuth({ KLIPY_API_KEY: KLIPY_KEY });
  assert.equal((await call("GET", "/commy/gifs/search", { query: { q: "wave" } })).status, 401);
  assert.equal(log.length, 0, "no upstream call without a session");

  const cookie = await signUp("gifs@example.test");
  const r = await call("GET", "/commy/gifs/search", { cookie, query: { q: "wave" } });
  assert.equal(r.status, 200, JSON.stringify(r.json));
  assert.equal(r.json.results.length, 1);
  assert.equal(r.json.provider, "klipy");
  assert.ok(!JSON.stringify(r.json).includes(KLIPY_KEY));
  assert.ok(log[0].url.startsWith(`https://api.klipy.com/api/v1/${KLIPY_KEY}/gifs/search?q=wave`));
  assert.match(log[0].url, /customer_id=[0-9a-f]{32}/);

  await call("GET", "/commy/gifs/search", { cookie });
  assert.ok(log[1].url.includes("/gifs/trending?"));
  await call("GET", "/commy/gifs/search", { cookie, query: { q: "WAVE" } });
  assert.equal(log.length, 2);
});

test("sticker search uses the KLIPY key server side", async () => {
  const { call, signUp, log } = makeAuth({ KLIPY_API_KEY: KLIPY_KEY });
  assert.equal((await call("GET", "/commy/stickers/search", { query: { q: "rawr" } })).status, 401);
  assert.equal(log.length, 0);

  const cookie = await signUp("stickers@example.test");
  const r = await call("GET", "/commy/stickers/search", { cookie, query: { q: "rawr" } });
  assert.equal(r.status, 200, JSON.stringify(r.json));
  assert.deepEqual(r.json.results.map((s) => s.id), ["grrr-rawr"]);
  assert.ok(!JSON.stringify(r.json).includes(KLIPY_KEY));
  assert.ok(log[0].url.startsWith(`https://api.klipy.com/api/v1/${KLIPY_KEY}/stickers/search?q=rawr`));
  assert.match(log[0].url, /format_filter=webp%2Cgif/);

  await call("GET", "/commy/stickers/search", { cookie });
  assert.ok(log[1].url.includes("/stickers/trending?"));
  await call("GET", "/commy/gifs/search", { cookie, query: { q: "rawr" } });
  assert.ok(log[2].url.includes("/gifs/search?"), "GIF and sticker results are cached apart");
});

test("game search reuses the Twitch app token", async () => {
  const { call, signUp, log } = makeAuth({ TWITCH_CLIENT_ID: "tw", TWITCH_CLIENT_SECRET: "tws" });
  const cookie = await signUp("games@example.test");
  assert.equal((await call("GET", "/commy/games/search", { query: { q: "portal" } })).status, 401);

  const r = await call("GET", "/commy/games/search", { cookie, query: { q: "portal" } });
  assert.equal(r.status, 200, JSON.stringify(r.json));
  assert.deepEqual(r.json.results, [{ id: 72, name: "Portal 2", cover: "co1rs4", year: 2011 }]);
  await call("GET", "/commy/games/search", { cookie, query: { q: "half life" } });
  assert.equal(log.filter((l) => l.url.startsWith("https://id.twitch.tv/")).length, 1, "one token for both searches");
  const igdb = log.find((l) => l.url.startsWith("https://api.igdb.com/"));
  assert.equal(igdb.init.headers["Client-ID"], "tw");
  assert.match(igdb.init.body, /^search "portal";/);

  const short = await call("GET", "/commy/games/search", { cookie, query: { q: "p" } });
  assert.deepEqual(short.json.results, []);
});

test("only GIF and cover image URLs are accepted for loading", () => {
  assert.ok(mediaUrl("https://static.klipy.com/ii/4e7b/fd/3b/5wGhb3j5.gif"));
  assert.ok(mediaUrl("https://images.igdb.com/igdb/image/upload/t_thumb/co1rs4.jpg"));
  assert.ok(mediaUrl("https://media2.giphy.com/media/v1.Y2lkPTc5MGI3NjExYWJjZGVm/200w.gif?cid=790b7611abcdef&ep=v1_gifs_search&rid=200w.gif&ct=g"));
  assert.ok(mediaUrl("https://media.giphy.com/media/3o7aD2saalBwwftBIY/giphy-downsized-medium.gif"));
  assert.ok(mediaUrl("https://media0.giphy.com/media/3o7aD2saalBwwftBIY/giphy.webp"));
  assert.ok(mediaUrl("https://i.giphy.com/3o7aD2saalBwwftBIY.gif"));
  assert.ok(mediaUrl("https://cdn3.emoji.gg/emojis/4384_falco_stare.png"));
  assert.ok(mediaUrl("https://cdn3.emoji.gg/emojis/9025-party-blob.gif"));
  assert.equal(mediaUrl("https://cdn3.emoji.gg/stickers/790374-smile.png"), null);
  assert.equal(mediaUrl("https://cdn3.emoji.gg/emojis/x.png?size=1"), null);
  for (const bad of [
    "https://media10.giphy.com/media/3o7aD2saalBwwftBIY/giphy.gif",
    "https://api.giphy.com/v1/gifs/search",
    "https://giphy.com/gifs/3o7aD2saalBwwftBIY",
    "https://media1.giphy.com/media/3o7aD2saalBwwftBIY/giphy.mp4",
    "https://media1.giphy.com/media/3o7aD2saalBwwftBIY/giphy.gif?" + "x".repeat(300),
    "https://media1.giphy.com/media/3o7aD2saalBwwftBIY/giphy.gif#frag",
    "https://user:pw@media1.giphy.com/media/3o7aD2saalBwwftBIY/giphy.gif",
    "http://static.klipy.com/a.gif",
    "https://api.klipy.com/api/v1/key/gifs/search",
    "https://static.klipy.com.evil.example/a.gif",
    "https://user:pw@static.klipy.com/a.gif",
    "https://static.klipy.com:8443/a.gif",
    "https://static.klipy.com/a%2F..%2Fb.gif",
    "https://images.igdb.com/igdb/image/upload/t_thumb/co1rs4.jpg?x=1",
    "https://images.igdb.com/other/path.jpg",
    "https://169.254.169.254/latest/meta-data",
    "file:///etc/passwd",
    "not a url",
  ]) {
    assert.equal(mediaUrl(bad), null, bad);
  }
});

test("media hosts match exactly, with one pattern for the numbered GIPHY hosts", () => {
  assert.equal(mediaHost("static.klipy.com").host, "static.klipy.com");
  for (const host of ["media.giphy.com", "media0.giphy.com", "media4.giphy.com", "i.giphy.com"]) assert.ok(mediaHost(host), host);
  for (const host of ["klipy.com", "static.klipy.com.evil.example", "xstatic.klipy.com", "media00.giphy.com", "giphy.com", ""]) {
    assert.equal(mediaHost(host), null, host);
  }
});

test("images load through the server for signed in apps only", async () => {
  const { call, signUp, log } = makeAuth();
  const gif = "https://static.klipy.com/ii/4e7b/fd/3b/5wGhb3j5.gif";
  assert.equal((await call("GET", "/commy/media", { query: { u: gif } })).status, 401);
  assert.equal(log.length, 0, "no upstream call without a session");

  const cookie = await signUp("media@example.test");
  const r = await call("GET", "/commy/media", { cookie, query: { u: gif } });
  assert.equal(r.status, 200);
  assert.equal(r.type, "image/gif");
  assert.deepEqual(r.bytes, GIF_BYTES);
  assert.equal(r.headers.get("x-content-type-options"), "nosniff");
  assert.equal(log[0].init.redirect, "error", "redirects never leave the allowed hosts");

  await call("GET", "/commy/media", { cookie, query: { u: gif } });
  assert.equal(log.length, 1);

  const cover = await call("GET", "/commy/media", { cookie, query: { u: "https://images.igdb.com/igdb/image/upload/t_thumb/co1rs4.jpg" } });
  assert.equal(cover.type, "image/jpeg");
  const giphy = await call("GET", "/commy/media", { cookie, query: { u: "https://media3.giphy.com/media/3o7aD2saalBwwftBIY/200w.gif?cid=790b7611abcdef&rid=200w.gif&ct=g" } });
  assert.equal(giphy.status, 200);
  assert.equal(giphy.type, "image/gif");

  assert.equal((await call("GET", "/commy/media", { cookie, query: { u: "https://example.org/a.gif" } })).status, 400);
  const before = log.length;
  assert.equal((await call("GET", "/commy/media", { cookie, query: { u: "https://static.klipy.com/page.html" } })).status, 502);
  assert.equal(log.length, before + 1);
});
