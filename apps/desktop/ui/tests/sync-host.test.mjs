import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { SYNC_HOST_MAX_CHARS, SYNC_HOST_INVALID, cleanSyncHost, syncHostNow, syncHostReady, syncHostAppNotice } = require("../scripts/sync-host.js");

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const core = readFileSync(join(repo, "crates", "commy-core", "src", "sync_host.rs"), "utf8");
const view = readFileSync(join(repo, "crates", "commy-view", "src", "sync_host.rs"), "utf8");
const html = readFileSync(join(import.meta.dirname, "..", "index.html"), "utf8");

test("the app and core agree on what a sync server address is", () => {
  assert.match(core, new RegExp(`MAX_SYNC_HOST_CHARS: usize = ${SYNC_HOST_MAX_CHARS};`));
  assert.ok(view.includes(`"${SYNC_HOST_INVALID}"`));
  for (const input of ["sync.example.org", " Sync.Example.org ", "https://sync.example.org/", "wss://sync.example.org/sync", "https://sync.example.org/sync/"]) {
    assert.equal(cleanSyncHost(input), "sync.example.org", input);
  }
  for (const input of ["", null, "http://sync.example.org", "ws://sync.example.org/sync", "javascript:alert(1)", "https://user@sync.example.org", "https://sync.example.org:8443", "https://sync.example.org/other", "https://localhost", "https://box.local", "https://10.0.0.4", "https://intranet", "https://-bad.example.org", `${"a".repeat(SYNC_HOST_MAX_CHARS)}.org`]) {
    assert.equal(cleanSyncHost(input), null, String(input));
  }
});

test("the page says where the server syncs now", () => {
  assert.equal(syncHostNow({ sync_host: null }), "This server syncs through Commy.chat.");
  assert.equal(syncHostNow({ sync_host: "sync.example.org" }), "This server syncs through sync.example.org.");
  assert.equal(syncHostNow({ sync_host: null }, null, "friends.example"), "This server syncs through friends.example.");
  assert.equal(syncHostNow({ sync_host: "sync.example.org" }, null, "friends.example"), "This server syncs through sync.example.org.");
});

test("the sync server page loads its script and is only for the owner's app", () => {
  assert.ok(html.indexOf('src="scripts/file-store.js"') < html.indexOf('src="scripts/sync-host.js"'));
  assert.match(html, /data-tab="sync-host" id="tab-sync-host"/);
  assert.match(html, /data-page="sync-host"/);
});

test("the page says which Commy the sync server runs", () => {
  const release = { version: "0.2.5", fit: "current", text: "It runs Commy 0.2.5." };
  assert.equal(syncHostNow({ sync_host: "sync.example.org" }, release), "This server syncs through sync.example.org. It runs Commy 0.2.5.");
  assert.equal(syncHostNow({ sync_host: null }, null), "This server syncs through Commy.chat.");
  assert.equal(syncHostReady("sync.example.org", release, false), "sync.example.org is ready. It runs Commy 0.2.5. Choose Move Now again to move this server there.");
  assert.equal(syncHostReady("Commy.chat", null, true), "Commy.chat is ready. Choose Move back to Commy.chat again to move this server there.");
});

test("members are told to update only when the sync server needs a newer app", () => {
  const behind = { fit: "app_behind", text: "This server's sync server needs a newer Commy. Update Commy to keep chatting here." };
  assert.equal(syncHostAppNotice(behind), behind.text);
  assert.ok(view.includes(`"${behind.text}"`));
  assert.equal(syncHostAppNotice({ fit: "host_behind", text: "x" }), null);
  assert.equal(syncHostAppNotice(null), null);
});
