import assert from "node:assert/strict";
import { mkdtemp, writeFile } from "node:fs/promises";
import http from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { toNodeHandler } from "better-auth/node";
import { sealedAtRest } from "../src/at-rest.js";
import { accessToken } from "../src/access.js";
import { buildAuthOptions } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";
import { createRequestHandler } from "../src/http.js";
import { createPages, escapeHtml, linksFromHeader, safeLink, scriptJson, SITE_LINKS_HEADER, siteLinks } from "../src/pages.js";
import { accountError, createLoaders, isPagePath, providersFrom } from "../pages/src/loaders.js";

const ORIGIN = "http://localhost:3001";
const ENV = {
  BETTER_AUTH_SECRET: "test-secret-".repeat(4),
  DATABASE_URL: "postgres://unused",
  BETTER_AUTH_URL: ORIGIN,
  COMMY_INSTANCE_NAME: "Pixel <Home>",
  ADULTS_ONLY: "true",
};
const PAGE_ENV = { COMMY_TERMS_URL: "/terms", COMMY_PRIVACY_URL: "https://example.test/privacy", COMMY_LOGO_URL: "javascript:alert(1)" };

let server;
let base;

function listen(handler) {
  const s = http.createServer(handler);
  return new Promise((resolve) => s.listen(0, "127.0.0.1", () => resolve(s)));
}

function buildAuth(config) {
  const options = buildAuthOptions(config, { pool: null, send: async () => {} });
  options.database = sealedAtRest(
    memoryAdapter({ user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], inviteBackground: [], doorbellRing: [], twoFactor: [], passkey: [] }),
    options.secret,
  );
  options.rateLimit = { enabled: false };
  return betterAuth(options);
}

async function startServer(env, pageEnv) {
  const config = loadConfig(env);
  const auth = buildAuth(config);
  const pages = createPages({ auth, config, discovery: discoveryDocument(config), env: pageEnv });
  assert.equal(pages.enabled, true, pages.reason);
  await pages.ready;
  const s = await listen(
    createRequestHandler({
      authHandler: toNodeHandler(auth),
      basePath: config.basePath,
      discovery: "{}",
      pool: { query: async () => {} },
      redis: null,
      pages,
    }),
  );
  return { server: s, base: `http://127.0.0.1:${s.address().port}` };
}

before(async () => {
  ({ server, base } = await startServer(ENV, PAGE_ENV));
});

after(() => server.close());

async function signUp(email) {
  const res = await fetch(`${base}/api/auth/sign-up/email`, {
    method: "POST",
    headers: { "content-type": "application/json", origin: ORIGIN },
    body: JSON.stringify({ email, password: "correct horse battery", name: "Tester", adultConfirmed: true }),
  });
  assert.equal(res.status, 200, await res.clone().text());
  return res.headers.getSetCookie()[0].split(";")[0];
}

async function page(path, { cookie, headers = {} } = {}) {
  const res = await fetch(`${base}${path}`, { headers: { ...(cookie ? { cookie } : {}), ...headers }, redirect: "manual" });
  return { status: res.status, headers: res.headers, body: await res.text() };
}

function pageData(body) {
  const match = body.match(/<script type="application\/json" id="commy-page-data">(.*?)<\/script>/s);
  return JSON.parse(match[1]);
}

test("the sign-in page carries the server's name, legal links and a safe logo", async () => {
  const res = await page("/account");
  assert.equal(res.status, 200);
  assert.match(res.headers.get("content-type"), /text\/html/);
  assert.equal(res.headers.get("cache-control"), "no-store, no-transform");
  assert.match(res.headers.get("content-security-policy"), /frame-ancestors 'none'/);
  assert.match(res.body, /<title>Account - Pixel &#60;Home&#62;<\/title>/);
  assert.match(res.body, /Sign in to Pixel &lt;Home>/);
  assert.match(res.body, /autocomplete="username webauthn"/);
  assert.match(res.body, />Continue</);
  assert.match(res.body, /class="tucked"/);
  assert.match(res.body, /<a href="\/terms">Terms<\/a>/);
  assert.match(res.body, /<a href="https:\/\/example.test\/privacy">Privacy<\/a>/);
  assert.doesNotMatch(res.body, /javascript:/);
  assert.match(res.body, /src="\/api\/auth\/commy\/pages\/logo\.svg\?v=/);
  const data = pageData(res.body);
  assert.equal(data.page, "account");
  assert.equal(data.props.user, null);
  assert.equal(data.site.name, "Pixel <Home>");
  assert.equal(data.site.host, "localhost:3001");
});

test("signed in, the account page lists the account and sign-up sends you on", async () => {
  const cookie = await signUp("pages@example.test");
  const account = await page("/account?notice=unlinked", { cookie });
  assert.match(account.body, /Your account/);
  assert.match(account.body, /pages@example\.test/);
  assert.match(account.body, /Login unlinked\./);
  const data = pageData(account.body);
  assert.deepEqual(data.props.accounts.map((a) => a.providerId), ["credential"]);
  assert.deepEqual(data.props.devices, []);
  assert.equal(data.props.user.email, "pages@example.test");
  assert.equal(Object.hasOwn(data.props.user, "emailVerified"), false);

  const signup = await page("/signup?next=/device", { cookie });
  assert.equal(signup.status, 303);
  assert.equal(signup.headers.get("location"), "/device");

  const away = await page("/account?next=//evil.test", { cookie });
  assert.equal(away.status, 200);
  const elsewhere = await page("/account?next=/delete-account", { cookie });
  assert.equal(elsewhere.headers.get("location"), "/delete-account");
});

test("every page renders", async () => {
  for (const [path, text] of [
    ["/signup", /Create your Pixel &lt;Home> account/],
    ["/account/two-factor?next=/device", /Two-step sign-in/],
    ["/account/reset-password?token=abc", /Save new password/],
    ["/account/reset-password?token=abc&error=INVALID_TOKEN", /This reset link has expired/],
    ["/account/verify-email?result=expired&next=/device", /This link has expired/],
    ["/delete-account", /Delete your Pixel &lt;Home> account/],
    ["/delete-account?notice=deleted", /Your account was deleted/],
    ["/device?user_code=abcd-1234", /Sign in first, then confirm the code/],
  ]) {
    const res = await page(path);
    assert.equal(res.status, 200, path);
    assert.match(res.body, text, path);
  }
});

test("forms stay shut until the page script takes over", async () => {
  for (const path of ["/account", "/signup", `/api/auth/commy/pages/render?path=${encodeURIComponent("/account")}`]) {
    const res = await page(path);
    const html = path.startsWith("/api/") ? JSON.parse(res.body).html : res.body;
    const fieldset = html.match(/<fieldset[^>]*class="page-fields"[^>]*>/)?.[0] ?? "";
    assert.match(fieldset, /\sdisabled[\s>]/, path);
    assert.match(fieldset, /display:\s*contents/, path);
  }
});

test("an unknown invite is a not found page", async () => {
  const res = await page("/i/no-such-invite");
  assert.equal(res.status, 404);
  assert.match(res.body, /Invite not found/);
  assert.equal(res.headers.get("x-robots-tag"), "noindex");
});

test("the render endpoint hands a website the page, its title and the site's own links", async () => {
  const res = await page(`/api/auth/commy/pages/render?path=${encodeURIComponent("/signup?next=/web/")}`, {
    headers: { [SITE_LINKS_HEADER]: "terms=/legal/terms&privacy=&home=javascript:alert(1)" },
  });
  assert.equal(res.status, 200);
  assert.equal(res.headers.get("cache-control"), "no-store, no-transform");
  const body = JSON.parse(res.body);
  assert.equal(body.title, "Create a Pixel <Home> account");
  assert.match(body.html, /href="\/legal\/terms"/);
  assert.doesNotMatch(body.html, /privacy policy/);
  assert.deepEqual(body.data.site.links, { home: "/", terms: "/legal/terms", privacy: null, download: "https://commy.chat/download" });
  assert.match(body.assets.script, /^\/api\/auth\/commy\/pages\/client\.js\?v=[0-9a-f]{16}$/);
  assert.match(body.assets.style, /^\/api\/auth\/commy\/pages\/pages\.css\?v=[0-9a-f]{16}$/);

  const cookie = await signUp("render@example.test");
  const redirect = JSON.parse((await page(`/api/auth/commy/pages/render?path=${encodeURIComponent("/signup?next=/web/")}`, { cookie })).body);
  assert.deepEqual(redirect, { redirect: "/web/" });

  const missing = await page(`/api/auth/commy/pages/render?path=${encodeURIComponent("/privacy")}`);
  assert.equal(missing.status, 404);
});

test("assets are cached for good only at their own version", async () => {
  const body = (await page("/account")).body;
  const script = body.match(/src="(\/api\/auth\/commy\/pages\/client\.js\?v=[0-9a-f]+)"/)[1];
  const fresh = await page(script);
  assert.equal(fresh.status, 200);
  assert.match(fresh.headers.get("content-type"), /text\/javascript/);
  assert.match(fresh.headers.get("cache-control"), /immutable/);
  const stale = await page("/api/auth/commy/pages/client.js?v=old");
  assert.equal(stale.headers.get("cache-control"), "public, max-age=300");
  const css = await page(body.match(/href="(\/api\/auth\/commy\/pages\/pages\.css\?v=[0-9a-f]+)"/)[1]);
  const font = css.body.match(/url\("?\.\/([a-z-]+-[A-Z0-9]+\.woff2)"?\)/)[1];
  const fontRes = await page(`/api/auth/commy/pages/${font}`);
  assert.equal(fontRes.headers.get("content-type"), "font/woff2");
  assert.match(fontRes.headers.get("cache-control"), /immutable/);
  for (const hidden of ["server.js", "assets.json", "..%2Fpackage.json"]) {
    assert.equal((await page(`/api/auth/commy/pages/${hidden}`)).status, 404, hidden);
  }
});

test("other paths still reach the auth handler", async () => {
  const res = await fetch(`${base}/api/auth/get-session`);
  assert.equal(res.status, 200);
  assert.equal((await page("/privacy")).status, 404);
});

test("with standalone pages off, only the render endpoint and assets answer", async () => {
  const off = await startServer(ENV, { COMMY_PAGES: "false" });
  try {
    assert.equal((await fetch(`${off.base}/account`)).status, 404);
    const rendered = await fetch(`${off.base}/api/auth/commy/pages/render?path=%2Faccount`);
    assert.equal(rendered.status, 200);
  } finally {
    off.server.close();
  }
});

test("sign-up behind an access code asks for it until the cookie is there", async () => {
  const locked = await startServer({ ...ENV, ACCESS_CODE: "Open Sesame" }, {});
  try {
    const gate = await fetch(`${locked.base}/signup`).then((r) => r.text());
    assert.match(gate, /Access code/);
    assert.equal(pageData(gate).props.locked, true);
    const open = await fetch(`${locked.base}/signup`, { headers: { cookie: `commy_access=${accessToken("open sesame")}` } }).then((r) => r.text());
    assert.equal(pageData(open).props.locked, false);
    assert.match(open, /Create account/);
  } finally {
    locked.server.close();
  }
});

test("a missing build turns the pages off with a reason", async () => {
  const empty = await mkdtemp(join(tmpdir(), "commy-pages-"));
  await writeFile(join(empty, "assets.json"), JSON.stringify({ "client.js": "x" }));
  const config = loadConfig(ENV);
  const pages = createPages({ auth: null, config, discovery: {}, env: {}, dist: empty });
  assert.equal(pages.enabled, false);
  assert.match(pages.reason, /pages\/dist/);
});

test("links accept same-site paths and web addresses only", () => {
  assert.equal(safeLink("/terms"), "/terms");
  assert.equal(safeLink("https://example.test/terms"), "https://example.test/terms");
  for (const bad of ["//evil.test", "javascript:alert(1)", "/a b", '/"x', "", null, "ftp://x"]) assert.equal(safeLink(bad), null, String(bad));
  assert.deepEqual(siteLinks({}), { home: "/", terms: null, privacy: null, download: "https://commy.chat/download" });
  const base = siteLinks({ COMMY_TERMS_URL: "/terms" });
  assert.equal(linksFromHeader(undefined, base), base);
  assert.deepEqual(linksFromHeader("home=&terms=//evil", base), { home: "/", terms: "/terms", privacy: null, download: "https://commy.chat/download" });
});

test("page data cannot close its script tag", () => {
  const json = scriptJson({ name: "</script><script>alert(1)</script> " });
  assert.doesNotMatch(json, /<\/script>/);
  assert.equal(JSON.parse(json).name, "</script><script>alert(1)</script> ");
  assert.equal(escapeHtml(`<a href="x">'&`), "&#60;a href=&#34;x&#34;&#62;&#39;&#38;");
});

test("page paths are the moved website routes", () => {
  for (const path of ["/account", "/signup", "/account/two-factor", "/account/reset-password", "/account/verify-email", "/delete-account", "/device", "/i/ABCD1234"]) {
    assert.equal(isPagePath(path), true, path);
  }
  for (const path of ["/", "/i/", "/i/a/b", "/accounts", "/web", "/privacy", "/api/auth/get-session"]) assert.equal(isPagePath(path), false, path);
});

test("loaders read only what each page shows", async () => {
  const asked = [];
  const answers = {
    "/get-session": { user: { id: "u1", name: "Ann", email: "ann@example.test", image: null, twoFactorEnabled: true, emailVerified: true } },
    "/commy/devices": { devices: [{ id: "d1", nodeId: "ab", name: "PC", createdAt: "2026-01-01", lastSeenAt: "2026-01-02", secret: "x" }] },
    "/list-accounts": [{ id: "a1", providerId: "credential", accessToken: "secret" }],
    "/passkey/list-user-passkeys": [{ id: "p1", name: "Laptop", createdAt: "2026-01-03", publicKey: "secret" }],
  };
  const load = createLoaders({
    fetchAuth: async (path) => {
      asked.push(path);
      return answers[path] ?? null;
    },
    name: "Home",
    providers: providersFrom({ providers: { google: true } }),
    hasAccess: () => true,
  });
  const account = await load("/account", new URLSearchParams("error=TOKEN_EXPIRED"), "");
  assert.equal(account.props.errorMessage, "That link has expired. Sign in to get a new one.");
  assert.deepEqual(account.props.devices, [{ id: "d1", nodeId: "ab", name: "PC", createdAt: "2026-01-01", lastSeenAt: "2026-01-02" }]);
  assert.deepEqual(account.props.accounts, [{ id: "a1", providerId: "credential" }]);
  assert.deepEqual(account.props.passkeys, [{ id: "p1", name: "Laptop", createdAt: "2026-01-03" }]);
  assert.deepEqual(account.props.user, { id: "u1", name: "Ann", email: "ann@example.test", image: null, twoFactorEnabled: true });

  asked.length = 0;
  const deleted = await load("/delete-account", new URLSearchParams("notice=deleted"), "");
  assert.deepEqual(asked, []);
  assert.equal(deleted.props.providers, null);
  assert.equal((await load("/nowhere", new URLSearchParams(), "")), null);
  assert.equal(accountError("SOMETHING_NEW"), "Sign-in did not complete, please try again.");
  assert.equal(accountError("constructor"), "Sign-in did not complete, please try again.");
  assert.equal(accountError(""), "");
});
