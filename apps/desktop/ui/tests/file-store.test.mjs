import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { FILE_STORE_MAX_CHARS, FILE_STORE_INVALID, cleanFileStore, fileStoreLabel, fileStoreStatus } = require("../scripts/file-store.js");

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const core = readFileSync(join(repo, "crates", "commy-core", "src", "file_store.rs"), "utf8");
const view = readFileSync(join(repo, "crates", "commy-view", "src", "file_store.rs"), "utf8");

test("the app and core agree on what a file storage address is", () => {
  assert.match(core, new RegExp(`MAX_FILE_STORE_CHARS: usize = ${FILE_STORE_MAX_CHARS};`));
  assert.ok(view.includes(`"${FILE_STORE_INVALID}"`));
  for (const input of [" https://commy-files.rockets.workers.dev ", "https://commy-files.rockets.workers.dev/", "https://Commy-Files.Rockets.workers.dev"]) {
    assert.equal(cleanFileStore(input), "https://commy-files.rockets.workers.dev", input);
  }
  assert.equal(cleanFileStore("https://files.example.org/commy/"), "https://files.example.org/commy");
  for (const input of ["", null, "http://files.example.org", "files.example.org", "javascript:alert(1)", "https://user@files.example.org", "https://files.example.org:8443", "https://files.example.org/x?y=1", "https://files.example.org/x#y", "https://files.example.org/../x", "https://localhost", "https://nas.local", "https://192.168.1.10", "https://[::1]", "https://intranet", "https://-bad.example.org", `https://${"a".repeat(FILE_STORE_MAX_CHARS)}.example.org`]) {
    assert.equal(cleanFileStore(input), null, String(input));
  }
});

test("the address reads without its scheme and the status says where files go", () => {
  assert.equal(fileStoreLabel("https://commy-files.rockets.workers.dev"), "commy-files.rockets.workers.dev");
  assert.equal(fileStoreLabel(null), "");
  assert.match(fileStoreStatus({ file_store: null }), /devices only/);
  assert.match(fileStoreStatus({ file_store: "https://commy-files.rockets.workers.dev" }), /commy-files\.rockets\.workers\.dev/);
});
