import { readdirSync, readFileSync } from "node:fs";
import { extname } from "node:path";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { nameTheClient, PRIVATE_NETWORKS, trustedProxyList } from "./client-ip.js";
import { hasAccess } from "./access.js";

export const PAGES_DIST = join(dirname(fileURLToPath(import.meta.url)), "..", "pages", "dist");
export const PAGE_ASSETS_PATH = "/commy/pages";
export const RENDER_PATH = `${PAGE_ASSETS_PATH}/render`;
export const SITE_LINKS_HEADER = "x-commy-site-links";
const DEFAULT_DOWNLOAD_URL = "https://commy.chat/download";
const LINK_NAMES = ["home", "terms", "privacy", "download"];
const FORWARDED_HEADERS = ["cookie", "x-forwarded-for", "user-agent"];
const ONE_YEAR = 31536000;
const NOT_FOUND = '{"error":"not_found"}';

const CONTENT_SECURITY_POLICY = [
  "default-src 'self'",
  "script-src 'self' https://challenges.cloudflare.com",
  "frame-src https://challenges.cloudflare.com",
  "connect-src 'self'",
  "img-src 'self' data: blob: https:",
  "style-src 'self' 'unsafe-inline'",
  "font-src 'self'",
  "base-uri 'none'",
  "form-action 'self'",
  "frame-ancestors 'none'",
].join("; ");

export function safeLink(value) {
  const link = String(value ?? "").trim();
  if (/^\/(?!\/)[^\s"<>\\]*$/.test(link)) return link;
  if (/^https?:\/\/[^\s"<>\\]+$/.test(link)) return link;
  return null;
}

export function siteLinks(env = process.env) {
  return {
    home: safeLink(env.COMMY_HOME_URL) ?? "/",
    terms: safeLink(env.COMMY_TERMS_URL),
    privacy: safeLink(env.COMMY_PRIVACY_URL),
    download: safeLink(env.COMMY_DOWNLOAD_URL) ?? DEFAULT_DOWNLOAD_URL,
  };
}

export function linksFromHeader(header, base) {
  if (!header) return base;
  const asked = new URLSearchParams(String(header));
  const links = { ...base };
  for (const name of LINK_NAMES) {
    if (!asked.has(name)) continue;
    const value = asked.get(name);
    links[name] = value === "" ? null : (safeLink(value) ?? links[name]);
  }
  links.home ??= "/";
  links.download ??= DEFAULT_DOWNLOAD_URL;
  return links;
}

export function escapeHtml(text) {
  return String(text).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}

export function scriptJson(value) {
  return JSON.stringify(value).replace(/</g, "\\u003c").replace(/\u2028/g, "\\u2028").replace(/\u2029/g, "\\u2029");
}

const CONTENT_TYPES = {
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
};
const SERVER_ONLY = new Set(["server.js", "assets.json"]);

function loadDist(dist) {
  const versions = JSON.parse(readFileSync(join(dist, "assets.json"), "utf8"));
  const files = {};
  for (const name of readdirSync(dist)) {
    if (SERVER_ONLY.has(name) || !Object.hasOwn(CONTENT_TYPES, extname(name))) continue;
    files[name] = { body: readFileSync(join(dist, name)), type: CONTENT_TYPES[extname(name)], version: versions[name] ?? null };
  }
  for (const name of Object.keys(versions)) {
    if (!files[name]) throw Object.assign(new Error(`${name} is missing`), { code: "ENOENT" });
  }
  return { files };
}

function headTags(loaded) {
  const tags = [`<title>${escapeHtml(loaded.title)}</title>`];
  if (loaded.description) tags.push(`<meta name="description" content="${escapeHtml(loaded.description)}">`);
  if (loaded.robots) tags.push(`<meta name="robots" content="${escapeHtml(loaded.robots)}">`);
  if (loaded.openGraph) {
    tags.push(`<meta property="og:title" content="${escapeHtml(loaded.openGraph.title)}">`);
    tags.push(`<meta property="og:site_name" content="${escapeHtml(loaded.openGraph.siteName)}">`);
    if (loaded.description) tags.push(`<meta property="og:description" content="${escapeHtml(loaded.description)}">`);
  }
  return tags.join("");
}

function footerLinks(site, consentSettings) {
  const items = [];
  if (site.links.terms) items.push(`<a href="${escapeHtml(site.links.terms)}">Terms</a>`);
  if (site.links.privacy) items.push(`<a href="${escapeHtml(site.links.privacy)}">Privacy</a>`);
  if (consentSettings) items.push('<button type="button" class="link-button" data-consent-settings>Cookie settings</button>');
  return items.join("");
}

export function pageDocument({ loaded, html, data, site, assets }) {
  const consentSettings = data.turnstile;
  return [
    '<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">',
    '<meta name="theme-color" content="#090a0e">',
    headTags(loaded),
    `<link rel="icon" href="${escapeHtml(site.logo)}">`,
    `<link rel="stylesheet" href="${escapeHtml(assets.style)}">`,
    "</head><body>",
    '<a href="#commy-page" class="skip-link">Skip to content</a>',
    `<header class="nav"><a href="${escapeHtml(site.links.home)}" class="wordmark"><img src="${escapeHtml(site.logo)}" alt="" width="32" height="32">${escapeHtml(site.name)}</a></header>`,
    `<div id="commy-page" data-standalone>${html}</div>`,
    `<footer class="pages-footer"><span>${escapeHtml(site.name)}</span>${footerLinks(site, consentSettings)}</footer>`,
    '<div id="commy-consent"></div>',
    `<script type="application/json" id="commy-page-data">${scriptJson(data)}</script>`,
    `<script src="${escapeHtml(assets.script)}" defer></script>`,
    "</body></html>",
  ].join("");
}

function send(res, status, headers, body) {
  res.writeHead(status, headers);
  res.end(body);
}

function noStore(extra = {}) {
  return { "cache-control": "no-store, no-transform", "x-content-type-options": "nosniff", ...extra };
}

export function createPages({ auth, config, discovery, env = process.env, dist = PAGES_DIST, trustedProxies = trustedProxyList(PRIVATE_NETWORKS), webApp = null }) {
  let built;
  try {
    built = { ...loadDist(dist), server: null };
  } catch (err) {
    return { enabled: false, reason: `account pages are off because pages/dist is missing (${err.code ?? err.message}); run node pages/build.mjs` };
  }

  const basePath = config.basePath;
  const assetBase = `${basePath}${PAGE_ASSETS_PATH}`;
  const assetUrl = (name) => `${assetBase}/${name}?v=${built.files[name].version}`;
  const links = siteLinks(env);
  const logo = safeLink(env.COMMY_LOGO_URL) ?? assetUrl("logo.svg");
  const host = new URL(config.baseURL).host;
  const standalone = env.COMMY_PAGES !== "false";
  const serverModule = import(join(dist, "server.js")).then((mod) => (built.server = mod));

  async function fetchAuth(req, path, cookies) {
    const headers = new Headers();
    for (const name of FORWARDED_HEADERS) {
      const value = req.headers[name];
      if (value) headers.set(name, Array.isArray(value) ? value.join(", ") : value);
    }
    const res = await auth.handler(new Request(`${config.baseURL}${basePath}${path}`, { headers }));
    for (const cookie of res.headers.getSetCookie?.() ?? []) cookies.push(cookie);
    if (!res.ok) return null;
    return res.json().catch(() => null);
  }

  async function renderFor(req, pathname, query, siteLinksFor) {
    const server = built.server ?? (await serverModule);
    const cookies = [];
    const providers = server.providersFrom(discovery, { adultsOnly: config.adultsOnly });
    const load = server.createLoaders({
      fetchAuth: (path) => fetchAuth(req, path, cookies),
      name: config.instanceName,
      providers,
      listing: config.listing?.enabled ?? false,
      hasAccess: (cookie) => hasAccess(cookie, config.accessCode),
    });
    const loaded = await load(pathname, query, req.headers.cookie ?? "");
    if (!loaded) return null;
    if (loaded.redirect) return { loaded, cookies };
    const site = { name: config.instanceName, host, logo, links: siteLinksFor, webApp };
    const data = { page: loaded.page, props: loaded.props, site, turnstile: Boolean(providers.turnstileSiteKey) };
    return { loaded, data, site, html: server.renderPage(data), cookies };
  }

  function assets() {
    return { script: assetUrl("client.js"), style: assetUrl("pages.css") };
  }

  async function servePage(req, res, url) {
    const rendered = await renderFor(req, url.pathname, url.searchParams, links);
    if (!rendered) return send(res, 404, noStore({ "content-type": "application/json" }), NOT_FOUND);
    const cookieHeaders = rendered.cookies.length ? { "set-cookie": rendered.cookies } : {};
    if (rendered.loaded.redirect) return send(res, 303, noStore({ location: rendered.loaded.redirect, ...cookieHeaders }), "");
    const body = pageDocument({ ...rendered, assets: assets() });
    send(
      res,
      rendered.loaded.status ?? 200,
      noStore({
        "content-type": "text/html; charset=utf-8",
        "content-security-policy": CONTENT_SECURITY_POLICY,
        "x-frame-options": "DENY",
        "referrer-policy": "strict-origin-when-cross-origin",
        ...(rendered.loaded.robots ? { "x-robots-tag": rendered.loaded.robots } : {}),
        ...cookieHeaders,
      }),
      body,
    );
  }

  async function serveRender(req, res, url) {
    const target = new URL(url.searchParams.get("path") ?? "", "http://pages.invalid");
    const askedLinks = linksFromHeader(req.headers[SITE_LINKS_HEADER], links);
    const rendered = await renderFor(req, target.pathname, target.searchParams, askedLinks);
    const json = (status, body) => send(res, status, noStore({ "content-type": "application/json" }), JSON.stringify(body));
    if (!rendered) return json(404, { error: "not_found" });
    const { loaded } = rendered;
    if (loaded.redirect) return json(200, { redirect: loaded.redirect });
    return json(200, {
      status: loaded.status ?? 200,
      title: loaded.title,
      description: loaded.description ?? null,
      robots: loaded.robots ?? null,
      openGraph: loaded.openGraph ?? null,
      html: rendered.html,
      data: rendered.data,
      assets: assets(),
    });
  }

  function serveAsset(req, res, name) {
    const file = Object.hasOwn(built.files, name) ? built.files[name] : null;
    if (!file) return send(res, 404, noStore({ "content-type": "application/json" }), NOT_FOUND);
    const asked = new URL(req.url ?? "/", "http://pages.invalid").searchParams.get("v");
    const versioned = file.version === null || asked === file.version;
    send(
      res,
      200,
      {
        "content-type": file.type,
        "cache-control": versioned ? `public, max-age=${ONE_YEAR}, immutable` : "public, max-age=300",
        "x-content-type-options": "nosniff",
      },
      file.body,
    );
  }

  return {
    enabled: true,
    ready: serverModule,
    handles(req) {
      if (req.method !== "GET" && req.method !== "HEAD") return false;
      const path = (req.url ?? "/").split("?")[0];
      if (path.startsWith(`${assetBase}/`)) return true;
      return standalone && Boolean(built.server?.isPagePath(path));
    },
    async respond(req, res) {
      const url = new URL(req.url ?? "/", "http://pages.invalid");
      nameTheClient(req, trustedProxies);
      if (url.pathname === `${basePath}${RENDER_PATH}`) return serveRender(req, res, url);
      if (url.pathname.startsWith(`${assetBase}/`)) return serveAsset(req, res, url.pathname.slice(assetBase.length + 1));
      return servePage(req, res, url);
    },
  };
}
