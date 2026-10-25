import assert from "node:assert/strict";
import http from "node:http";
import { test } from "node:test";
import { discoveryDocument, featureSummary, loadConfig } from "../src/config.js";
import { createRequestHandler } from "../src/http.js";
import {
  DEFAULT_LINK_BLOCKLIST_SOURCES,
  blocklistDomains,
  createLinkBlocklist,
  linkBlocklistSources,
} from "../src/link-blocklist.js";

const ENV = { BETTER_AUTH_SECRET: "test-secret-".repeat(4), DATABASE_URL: "postgres://unused", BETTER_AUTH_URL: "https://chat.example.org" };
const quiet = { warn: () => {} };

function textReply(body, status = 200) {
  return new Response(body, { status, headers: { "content-type": "text/plain" } });
}

function fakeFetch(bodies) {
  return async (url) => {
    const body = bodies[url];
    if (body instanceof Error) throw body;
    return body === undefined ? textReply("", 404) : textReply(body);
  };
}

async function serve(linkBlocklist) {
  const handler = createRequestHandler({
    authHandler: (req, res) => {
      res.writeHead(418);
      res.end();
    },
    basePath: "/api/auth",
    discovery: "{}",
    pool: { query: async () => {} },
    redis: null,
    linkBlocklist,
  });
  const server = http.createServer(handler);
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return { url: `http://127.0.0.1:${server.address().port}/api/auth/commy/link-blocklist`, close: () => server.close() };
}

test("list lines in hosts, filter and plain form become domains", () => {
  const domains = blocklistDomains("# Title\n! note\n0.0.0.0 Bad.Example\n||worse.example^\nplain.example.\n\nlocalhost\n<html>\n");
  assert.deepEqual([...domains].sort(), ["bad.example", "plain.example", "worse.example"]);
});

test("sources default to the public lists, can be replaced and turned off", () => {
  assert.deepEqual(linkBlocklistSources(undefined), DEFAULT_LINK_BLOCKLIST_SOURCES);
  assert.deepEqual(linkBlocklistSources("https://a.example/list.txt, http://insecure.example/x"), ["https://a.example/list.txt"]);
  assert.deepEqual(linkBlocklistSources("off"), []);
});

test("discovery names the list only while it is on", () => {
  const on = discoveryDocument(loadConfig(ENV));
  assert.equal(on.endpoints.linkBlocklist, "https://chat.example.org/api/auth/commy/link-blocklist");
  assert.equal(on.features.linkBlocklist, true);
  assert.match(featureSummary(loadConfig(ENV)), /link_blocklist=on/);

  const off = discoveryDocument(loadConfig({ ...ENV, LINK_BLOCKLIST_URLS: "off" }));
  assert.equal(off.endpoints.linkBlocklist, undefined);
  assert.equal(off.features.linkBlocklist, false);
});

test("the whole merged list is served with an etag, and a failed source keeps its last copy", async () => {
  const bodies = { "https://a.example/one": "one.example\nshared.example\n", "https://b.example/two": "two.example\nshared.example\n" };
  const list = createLinkBlocklist({ sources: Object.keys(bodies), fetch: fakeFetch(bodies), log: quiet });
  const site = await serve(list);
  try {
    assert.equal((await fetch(site.url)).status, 503);

    assert.equal(await list.refresh(), true);
    const first = await fetch(site.url);
    assert.equal(first.status, 200);
    assert.match(first.headers.get("cache-control"), /max-age=/);
    assert.equal(await first.text(), "one.example\nshared.example\ntwo.example\n");

    const again = await fetch(site.url, { headers: { "if-none-match": first.headers.get("etag") } });
    assert.equal(again.status, 304);

    bodies["https://b.example/two"] = new Error("offline");
    assert.equal(await list.refresh(), false);
    assert.equal(await (await fetch(site.url)).text(), "one.example\nshared.example\ntwo.example\n");
    assert.equal(list.count(), 3);
  } finally {
    site.close();
  }
});

test("a list that is off leaves the path to the account routes", async () => {
  const list = createLinkBlocklist({ sources: [], fetch: fakeFetch({}), log: quiet });
  const site = await serve(list);
  try {
    assert.equal((await fetch(site.url)).status, 418);
  } finally {
    site.close();
  }
});
