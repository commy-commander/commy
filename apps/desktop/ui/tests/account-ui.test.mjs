import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { UI_OWNER_KEY, ACCOUNT_UI_KEYS, claimUiState, unclaimedProfileLine, accountRows, canAddAccount, accountLine, signOutPrompt, accountChangeStep } = require("../scripts/account-ui.js");
const LOCALE_EN = require("../scripts/locales/en.js");

function memoryStorage(entries = {}) {
  const map = new Map(Object.entries(entries));
  return {
    getItem: key => (map.has(key) ? map.get(key) : null),
    setItem: (key, value) => map.set(key, String(value)),
    removeItem: key => map.delete(key),
    keys: () => [...map.keys()],
  };
}

const t = (key, vars) => String(LOCALE_EN[key]).replace(/\{(\w+)\}/g, (_, name) => String(vars[name]));

test("another account's remembered ids and camera picture are forgotten", () => {
  const storage = memoryStorage({
    "commy.pins.seen": "{}",
    "commy.hiddenMessages": "[\"m1\"]",
    "commy.cameraBackdrop": "data:image/png;base64,AAAA",
    "commy.appearance": "{\"theme\":\"dark\"}",
    [UI_OWNER_KEY]: "user-a",
  });
  assert.equal(claimUiState(storage, "user-b"), 3);
  assert.equal(storage.getItem("commy.hiddenMessages"), null);
  assert.equal(storage.getItem("commy.cameraBackdrop"), null);
  assert.equal(storage.getItem("commy.appearance"), "{\"theme\":\"dark\"}", "device preferences stay");
  assert.equal(storage.getItem(UI_OWNER_KEY), "user-b");
});

test("the same account keeps its state, and storage from before owners is cleared once", () => {
  const storage = memoryStorage({ "commy.switcher.recent": "[\"s1\"]" });
  assert.equal(claimUiState(storage, "user-a"), 1);
  storage.setItem("commy.switcher.recent", "[\"s2\"]");
  assert.equal(claimUiState(storage, "user-a"), 0);
  assert.equal(storage.getItem("commy.switcher.recent"), "[\"s2\"]");
  assert.equal(claimUiState(storage, ""), 0);
  assert.equal(claimUiState(null, "user-a"), 0);
});

test("every account-only key is one the app really uses", () => {
  const sources = ["app.js", "camera-pipeline.js", "sensitive.js"]
    .map(name => require("node:fs").readFileSync(new URL(`../scripts/${name}`, import.meta.url), "utf8"))
    .join("\n");
  for (const key of ACCOUNT_UI_KEYS) assert.ok(sources.includes(`"${key}"`), key);
});

test("the unclaimed profile reads as one line", () => {
  assert.equal(unclaimedProfileLine({ username: "ferris", display_name: "Ferris", servers: 3 }, t), "Ferris (@ferris) · 3 servers");
  assert.equal(unclaimedProfileLine({ username: "ferris", display_name: null, servers: 1 }, t), "ferris · 1 server");
  assert.equal(unclaimedProfileLine({ username: "", display_name: null, servers: 0 }, t), "Unnamed profile · 0 servers");
  assert.equal(unclaimedProfileLine(null, t), "");
});

const signedIn = others => ({
  signed_in: true,
  user: { id: "a", name: "Main", email: "main@example.test" },
  server: "https://commy.chat",
  server_host: "commy.chat",
  others,
  max_accounts: 3,
});
const other = (id, server = "https://commy.chat") => ({ id, name: id, email: `${id}@example.test`, server, server_host: new URL(server).host });

test("the account list starts with the account in use", () => {
  const rows = accountRows(signedIn([other("b"), other("c")]));
  assert.deepEqual(rows.map(r => [r.id, r.current]), [["a", true], ["b", false], ["c", false]]);
  assert.deepEqual(accountRows({ signed_in: false, others: [] }), []);
  assert.deepEqual(accountRows({ signed_in: true, user: { id: "a" } }), [], "a client without the switcher shows no list");
});

test("accounts can be added up to the limit", () => {
  assert.equal(canAddAccount(signedIn([])), true);
  assert.equal(canAddAccount(signedIn([other("b")])), true);
  assert.equal(canAddAccount(signedIn([other("b"), other("c")])), false);
  assert.equal(canAddAccount({ signed_in: false }), false);
});

test("an account on another server names it", () => {
  const account = signedIn([]);
  assert.equal(accountLine(other("b"), account), "b@example.test");
  assert.equal(accountLine(other("b", "https://chat.example.org"), account), "b@example.test · chat.example.org");
});

test("signing out says where Commy goes next", () => {
  assert.match(signOutPrompt(signedIn([other("b")]), t), /restarts into b,/);
  assert.match(signOutPrompt(signedIn([]), t), /sign-in screen/);
});

test("a first sign-in opens the account, and later changes never cover it with the account list", () => {
  const signedIn = { signed_in: true, user: { id: "u1" } };
  assert.equal(accountChangeStep(false, signedIn, false), "open");
  assert.equal(accountChangeStep(true, signedIn, false), null);
  assert.equal(accountChangeStep(true, signedIn, true), "accounts");
});

test("signing out or a waiting sign-in code shows the sign-in screen", () => {
  assert.equal(accountChangeStep(true, { signed_in: false }, false), "sign-in");
  assert.equal(accountChangeStep(false, { signed_in: false, pending_code: "ABCD" }, false), "sign-in");
  assert.equal(accountChangeStep(false, { signed_in: false }, false), null);
});
