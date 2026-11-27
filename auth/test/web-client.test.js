import assert from "node:assert/strict";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import http from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, test } from "node:test";
import { configuredRelays, createWebClient, isPhone, pageWithRelays, WEB_CLIENT_CONTENT_SECURITY_POLICY, withQuery } from "../src/web-client.js";

const PAGE = `<!doctype html><html><head>
  <base href="/web/" />
  <meta name="commy-relays" content="https://relay.commy.chat:8443" />
</head><body><script src="scripts/app.js"></script></body></html>`;
const DESKTOP = "Mozilla/5.0 (X11; Linux x86_64) Chrome/140.0 Safari/537.36";
const IPHONE = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) Mobile/15E148";

let server;
let base;
let dir;

before(async () => {
  dir = await mkdtemp(join(tmpdir(), "commy-web-"));
  await mkdir(join(dir, "scripts", "pkg"), { recursive: true });
  await writeFile(join(dir, "index.html"), PAGE);
  await writeFile(join(dir, "scripts", "app.js"), "console.log(1)");
  await writeFile(join(dir, "scripts", "pkg", "client_bg.wasm"), Buffer.from([0, 97, 115, 109]));
  await writeFile(join(dir, "scripts", "with space.js"), "1");
  await mkdir(join(dir, "assets"), { recursive: true });
  await writeFile(join(dir, "assets", "scripts-0123456789abcdef.js"), "console.log(2)");
  const web = createWebClient({ dir, env: { COMMY_RELAY_URLS: "https://relay.example.test:8443/", COMMY_DOWNLOAD_URL: "/download" } });
  assert.equal(web.enabled, true, web.reason);
  server = http.createServer((req, res) => {
    if (web.handles(req)) return web.respond(req, res);
    res.writeHead(418);
    res.end();
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  base = `http://127.0.0.1:${server.address().port}`;
});

after(() => server.close());

function rawStatus(path) {
  return new Promise((resolve, reject) => {
    http.get({ host: "127.0.0.1", port: server.address().port, path }, (res) => {
      res.resume();
      resolve(res.statusCode);
    }).on("error", reject);
  });
}

function get(path, headers = {}, method = "GET") {
  return fetch(`${base}${path}`, { headers: { "user-agent": DESKTOP, ...headers }, method, redirect: "manual" });
}

test("the web app page carries the server's relay and the site's safety headers", async () => {
  for (const path of ["/web", "/web/", "/web/index.html"]) {
    const res = await get(path);
    assert.equal(res.status, 200, path);
    assert.equal(res.headers.get("content-type"), "text/html; charset=utf-8");
    assert.equal(res.headers.get("cache-control"), "no-cache");
    assert.equal(res.headers.get("x-frame-options"), "DENY");
    assert.equal(res.headers.get("vary"), "User-Agent");
    const body = await res.text();
    assert.match(body, /<meta name="commy-relays" content="https:\/\/relay\.example\.test:8443" \/>/);
    assert.doesNotMatch(body, /relay\.commy\.chat/);
  }
});

function directives(policy) {
  return new Map(policy.split(";").map((part) => part.trim().split(/\s+/)).filter(([name]) => name).map(([name, ...values]) => [name, values]));
}

test("the web app page and its files only run the app's own scripts", async () => {
  for (const path of ["/web/", "/web/scripts/app.js", "/web/scripts/pkg/client_bg.wasm"]) {
    const res = await get(path);
    assert.equal(res.headers.get("content-security-policy"), WEB_CLIENT_CONTENT_SECURITY_POLICY, path);
  }
  const policy = directives(WEB_CLIENT_CONTENT_SECURITY_POLICY);
  assert.deepEqual(policy.get("script-src"), ["'self'", "'wasm-unsafe-eval'"]);
  assert.deepEqual(policy.get("object-src"), ["'none'"]);
  assert.deepEqual(policy.get("frame-ancestors"), ["'none'"]);
  assert.deepEqual(policy.get("base-uri"), ["'self'"]);
  for (const values of policy.values()) {
    assert.ok(!values.includes("'unsafe-inline'") || policy.get("style-src") === values, "only styles may be inline");
    assert.ok(!values.includes("'unsafe-eval'"));
  }
});

test("phones are sent to the download page", async () => {
  const res = await get("/web", { "user-agent": IPHONE });
  assert.equal(res.status, 307);
  assert.equal(res.headers.get("location"), "/download?from=web");
  assert.equal((await get("/web", { "user-agent": "Mozilla/5.0 (Linux; Android 15)" })).status, 307);
});

test("files are served with their type and revalidate by etag", async () => {
  const js = await get("/web/scripts/app.js");
  assert.equal(js.status, 200);
  assert.equal(js.headers.get("content-type"), "text/javascript; charset=utf-8");
  assert.equal(js.headers.get("cache-control"), "public, max-age=0, must-revalidate");
  assert.equal(await js.text(), "console.log(1)");
  const again = await get("/web/scripts/app.js", { "if-none-match": js.headers.get("etag") });
  assert.equal(again.status, 304);
  const wasm = await get("/web/scripts/pkg/client_bg.wasm");
  assert.equal(wasm.headers.get("content-type"), "application/wasm");
  assert.deepEqual([...new Uint8Array(await wasm.arrayBuffer())], [0, 97, 115, 109]);
  assert.equal((await get("/web/scripts/with%20space.js")).status, 200);
  const page = await get("/web");
  assert.equal((await get("/web", { "if-none-match": page.headers.get("etag") })).status, 304);
});

test("hashed bundle files are cached for a year and never revalidated", async () => {
  const res = await get("/web/assets/scripts-0123456789abcdef.js");
  assert.equal(res.status, 200);
  assert.equal(res.headers.get("cache-control"), "public, max-age=31536000, immutable");
  assert.equal((await get("/web/scripts/app.js")).headers.get("cache-control"), "public, max-age=0, must-revalidate");
});

test("head requests send no body", async () => {
  const res = await get("/web/scripts/app.js", {}, "HEAD");
  assert.equal(res.status, 200);
  assert.equal(res.headers.get("content-length"), "14");
  assert.equal(await res.text(), "");
});

test("only files from the build are served", async () => {
  for (const path of ["/web/missing.js", "/web/../package.json", "/web/%2e%2e/package.json", "/web/scripts/..%2f..%2fpackage.json", "/web/%E0%A4%A"]) {
    assert.equal(await rawStatus(path), 404, path);
  }
  assert.equal((await get("/webby")).status, 418);
  assert.equal((await fetch(`${base}/web/scripts/app.js`, { method: "POST" })).status, 418);
});

test("without a build the web app is off with a reason", async () => {
  const web = createWebClient({ dir: join(dir, "nothing-here") });
  assert.equal(web.enabled, false);
  assert.match(web.reason, /the web app is off/);
});

test("relays must be web addresses", () => {
  assert.equal(configuredRelays({}), null);
  assert.equal(configuredRelays({ COMMY_RELAY_URLS: " , " }), null);
  assert.equal(configuredRelays({ COMMY_RELAY_URLS: 'https://x" /><script>' }), null);
  assert.equal(configuredRelays({ COMMY_RELAY_URLS: "javascript:alert(1)" }), null);
  assert.equal(configuredRelays({ COMMY_RELAY_URLS: "https://a.test:8443/, https://b.test:8443" }), "https://a.test:8443,https://b.test:8443");
  assert.equal(pageWithRelays(PAGE, null), PAGE);
  assert.match(pageWithRelays(PAGE, "https://relay.test/$&$'$`"), /content="https:\/\/relay\.test\/\$&\$'\$`"/);
});

test("phones are told apart from computers", () => {
  assert.equal(isPhone(IPHONE), true);
  assert.equal(isPhone("Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X)"), true);
  assert.equal(isPhone(DESKTOP), false);
  assert.equal(isPhone(undefined), false);
});

test("the download link keeps its own query", () => {
  assert.equal(withQuery("https://commy.chat/download", "from", "web"), "https://commy.chat/download?from=web");
  assert.equal(withQuery("/download?os=mac#top", "from", "web"), "/download?os=mac&from=web#top");
});
