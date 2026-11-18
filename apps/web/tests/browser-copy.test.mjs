import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { WEB_COPY_DB, webBrowserCopy } = require("../ui/web-commands.js");

const repo = join(import.meta.dirname, "..", "..", "..");
const read = (...path) => readFileSync(join(repo, ...path), "utf8").replace(/\r\n/g, "\n");

function fakeIndexedDB({ failWrites = false } = {}) {
  const databases = new Map();
  const later = fn => setTimeout(fn, 0);
  function connect(data) {
    return {
      closed: false,
      createObjectStore() {},
      close() {
        this.closed = true;
      },
      transaction(_store, mode) {
        const tx = {};
        const pending = [];
        const answer = value => {
          const request = {};
          pending.push(() => {
            request.result = value();
          });
          return request;
        };
        tx.objectStore = () => ({
          getAllKeys: () => answer(() => [...data.keys()]),
          getAll: () => answer(() => [...data.values()]),
          put: (value, key) => pending.push(() => data.set(key, value)),
          delete: key => pending.push(() => data.delete(key)),
        });
        later(() => {
          if (failWrites && mode === "readwrite") return tx.onabort && tx.onabort();
          pending.forEach(step => step());
          tx.oncomplete && tx.oncomplete();
        });
        return tx;
      },
    };
  }
  return {
    databases,
    open(name) {
      const request = {};
      later(() => {
        const fresh = !databases.has(name);
        if (fresh) databases.set(name, new Map());
        request.result = connect(databases.get(name));
        if (fresh && request.onupgradeneeded) request.onupgradeneeded();
        request.onsuccess && request.onsuccess();
      });
      return request;
    },
    deleteDatabase(name) {
      const request = {};
      later(() => {
        databases.delete(name);
        request.onsuccess && request.onsuccess();
      });
      return request;
    },
  };
}

test("the browser copy keeps what it is given and forgets what is set to null", async () => {
  const idb = fakeIndexedDB();
  const copy = webBrowserCopy(idb);
  assert.equal(await copy.write(JSON.stringify({ account: "sealed1.a", "server-1": "sealed1.b" })), true);
  assert.equal(await copy.write(JSON.stringify({ "server-1": null, "server-2": "sealed1.c" })), true);
  assert.deepEqual(JSON.parse(await copy.load()), { account: "sealed1.a", "server-2": "sealed1.c" });
});

test("a browser without storage opens with an empty copy and writes nothing", async () => {
  const copy = webBrowserCopy(null);
  assert.equal(await copy.load(), "{}");
  assert.equal(await copy.write(JSON.stringify({ account: "x" })), false);
  assert.equal(await copy.clear(), false);
});

test("a failed write says so, so the app can send it again", async () => {
  const copy = webBrowserCopy(fakeIndexedDB({ failWrites: true }));
  assert.equal(await copy.write(JSON.stringify({ account: "x" })), false);
  assert.equal(await copy.write("not json"), false);
});

test("signing out deletes the whole copy", async () => {
  const idb = fakeIndexedDB();
  const copy = webBrowserCopy(idb);
  await copy.write(JSON.stringify({ account: "sealed1.a" }));
  assert.ok(idb.databases.has(WEB_COPY_DB));
  assert.equal(await copy.clear(), true);
  assert.ok(!idb.databases.has(WEB_COPY_DB));
});

test("clearing for a signed-out visitor never creates the copy", async () => {
  const idb = fakeIndexedDB();
  const copy = webBrowserCopy(idb);
  assert.equal(await copy.clear(), true);
  assert.equal(idb.databases.size, 0);
});

test("the bridge hands the copy to the app and saves it when the tab is hidden or closed", () => {
  const bridge = read("apps", "web", "ui", "web-bridge.js");
  assert.match(bridge, /webBrowserCopy\(/);
  assert.match(bridge, /copyLoad: \(\) => browserCopy\.load\(\)/);
  assert.match(bridge, /copyWrite: changes => browserCopy\.write\(changes\)/);
  assert.match(bridge, /copyClear: \(\) => browserCopy\.clear\(\)/);
  assert.match(bridge, /backend\("web_keep_copy"\)/);
  const session = read("apps", "web", "src", "session.rs");
  assert.match(session, /"web_keep_copy" =>/);
  assert.match(session, /"web_farewell" => \{\n\s+self\.keep_copy\(\);/);
});

test("every way out of the account wipes the copy", () => {
  const session = read("apps", "web", "src", "session.rs");
  const leave = /async fn leave_account\(&self\) \{([\s\S]*?)\n    \}/.exec(session)[1];
  assert.match(leave, /self\.forget_copy\(\)\.await/);
  const deletion = /async fn delete_account_finish\([\s\S]*?\n    \}/.exec(session)[0];
  assert.match(deletion, /self\.forget_copy\(\)\.await/);
  const signedOut = /Unlock::SignedOut, "Sign in first"\);\n\s+self\.forget_copy\(\)\.await;/;
  assert.match(session, signedOut);
});
