import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { SYNC_HOST_MAX_CHARS, SYNC_HOST_INVALID, cleanSyncHost, syncHostHomeButton, syncHostNow, syncHostReady, syncHostAppNotice } = require("../scripts/sync-host.js");

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const core = readFileSync(join(repo, "crates", "commy-core", "src", "sync_host.rs"), "utf8");
const view = readFileSync(join(repo, "crates", "commy-view", "src", "sync_host.rs"), "utf8");
const html = readFileSync(join(import.meta.dirname, "..", "index.html"), "utf8");
const OFFICIAL = "Right now this server syncs through Commy.chat, Commy's official sync server.";

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
  assert.equal(syncHostNow({ sync_host: null }), OFFICIAL);
  assert.equal(syncHostNow({ sync_host: null }, null, "commy.chat"), OFFICIAL);
  assert.equal(syncHostNow({ sync_host: "sync.example.org" }), "Right now this server syncs through sync.example.org.");
  assert.equal(syncHostNow({ sync_host: null }, null, "friends.example"), "Right now this server syncs through friends.example.");
  assert.equal(syncHostNow({ sync_host: "sync.example.org" }, null, "friends.example"), "Right now this server syncs through sync.example.org.");
});

test("the sync server page loads its script and is only for the owner's app", () => {
  assert.ok(html.indexOf('src="scripts/file-store.js"') < html.indexOf('src="scripts/sync-host.js"'));
  assert.match(html, /data-tab="sync-host" id="tab-sync-host"/);
  assert.match(html, /data-page="sync-host"/);
});

test("the page says which Commy the sync server runs", () => {
  const release = { version: "0.2.5", fit: "current", text: "It runs Commy 0.2.5." };
  assert.equal(syncHostNow({ sync_host: "sync.example.org" }, release), "Right now this server syncs through sync.example.org. It runs Commy 0.2.5.");
  assert.equal(syncHostNow({ sync_host: null }, null), OFFICIAL);
  assert.equal(syncHostReady("sync.example.org", release, false), "sync.example.org is ready. It runs Commy 0.2.5. Choose Move Now to move this server there.");
  assert.equal(syncHostReady("Commy.chat", null, true), "Commy.chat is ready. Choose Move back to Commy.chat again to move it back.");
});

test("the page names who runs the sync server when it has a name", () => {
  const dev = { version: "main", fit: "current", text: "It runs a development build of Commy.", operator: "Dev test host" };
  assert.equal(syncHostNow({ sync_host: null }, dev, "dev.commy.chat"), "Right now this server syncs through Dev test host at dev.commy.chat. It runs a development build of Commy.");
  assert.equal(syncHostNow({ sync_host: "sync.example.org" }, { ...dev, operator: "Sync.Example.org" }), "Right now this server syncs through sync.example.org. It runs a development build of Commy.");
  assert.equal(syncHostNow({ sync_host: "sync.example.org" }, { ...dev, operator: null }), "Right now this server syncs through sync.example.org. It runs a development build of Commy.");
  assert.ok(view.includes("pub operator: Option<String>"), "the app reads the name the sync server sends");
});

test("moving back names the owner's own account server", () => {
  assert.equal(syncHostHomeButton("commy.chat"), "Move back to Commy.chat");
  assert.equal(syncHostHomeButton("staging.commy.chat"), "Move back to staging.commy.chat");
  assert.equal(syncHostHomeButton(null), "Move back to Commy.chat");
  assert.equal(syncHostReady("staging.commy.chat", null, true), "staging.commy.chat is ready. Choose Move back to staging.commy.chat again to move it back.");
  assert.doesNotMatch(html, /Move back to Commy\.chat/, "the button is named from the owner's account server, not fixed text");
});

test("members are told to update only when the sync server needs a newer app", () => {
  const behind = { fit: "app_behind", text: "This server's sync server needs a newer Commy. Update Commy to keep chatting here." };
  assert.equal(syncHostAppNotice(behind), behind.text);
  assert.ok(view.includes(`"${behind.text}"`));
  assert.equal(syncHostAppNotice({ fit: "host_behind", text: "x" }), null);
  assert.equal(syncHostAppNotice(null), null);
});

test("the official sync server never tells an owner to update it themselves", () => {
  const older = { version: null, fit: "host_behind", text: "It runs an earlier Commy, which is older than this app. Ask whoever runs it to update it, or run setup.sh again if it's yours." };
  assert.equal(syncHostNow({ sync_host: null }, older), OFFICIAL);
  assert.equal(syncHostNow({ sync_host: null }, { version: "1.0.0", fit: "current", text: "It runs Commy 1.0.0." }), OFFICIAL);
  assert.match(syncHostNow({ sync_host: "sync.example.org" }, older), /run setup\.sh again/, "a community's own sync server still gets the hint");
  const appBehind = { fit: "app_behind", text: "This server's sync server needs a newer Commy. Update Commy to keep chatting here." };
  assert.equal(syncHostNow({ sync_host: null }, appBehind), `${OFFICIAL} ${appBehind.text}`);
});

test("the sync server page explains moving once, in steps", () => {
  const page = html.slice(html.indexOf('data-page="sync-host"'), html.indexOf('data-page="file-storage"'));
  assert.equal((page.match(/<li>/g) || []).length, 4);
  assert.equal((page.match(/1,000/g) || []).length, 1, "the history limit is said once");
  assert.doesNotMatch(page, /sync-host-guide/);
  assert.match(page, /id="sync-host-save">Check</);
});
