// GIF and game search run with the server's keys, for signed in apps only.
import assert from "node:assert/strict";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, featureSummary, loadConfig } from "../src/config.js";
import { apicalypseString, cleanQuery, customerId, igdbResults, klipyResults } from "../src/media.js";

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

function fakeFetch(log) {
  return async (url, init = {}) => {
    const u = String(url);
    log.push({ url: u, init });
    if (u.startsWith("https://api.klipy.com/")) return Response.json(klipyBody);
    if (u.startsWith("https://id.twitch.tv/")) return Response.json({ access_token: "twitch-token", expires_in: 3600 });
    if (u.startsWith("https://api.igdb.com/")) {
      return Response.json([{ id: 72, name: "Portal 2", cover: { image_id: "co1rs4" }, first_release_date: 1303171200 }]);
    }
    return new Response("not found", { status: 404 });
  };
}

function makeAuth(extraEnv = {}) {
  const log = [];
  const options = buildAuthOptions(loadConfig({ ...ENV, ...extraEnv }), { pool: null, send: async () => {}, fetch: fakeFetch(log) });
  const db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [] };
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
  assert.deepEqual(none.features, { gifs: false, games: false });
  const both = discoveryDocument(loadConfig({ ...ENV, KLIPY_API_KEY: KLIPY_KEY, TWITCH_CLIENT_ID: "id", TWITCH_CLIENT_SECRET: "s" }));
  assert.deepEqual(both.features, { gifs: true, games: true });
  assert.equal(both.endpoints.gifSearch, `${ORIGIN}/api/auth/commy/gifs/search`);
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

  // Trending for an empty query, and repeats come from the cache
  await call("GET", "/commy/gifs/search", { cookie });
  assert.ok(log[1].url.includes("/gifs/trending?"));
  await call("GET", "/commy/gifs/search", { cookie, query: { q: "WAVE" } });
  assert.equal(log.length, 2);
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
