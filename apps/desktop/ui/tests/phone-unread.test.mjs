import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const app = readFileSync(new URL("../scripts/app.js", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const fn = name => {
  const start = app.indexOf(`function ${name}(`);
  assert.ok(start >= 0, name);
  return app.slice(start, app.indexOf("\n}\n", start));
};

test("on a phone's list the last open chat is not being read, so new messages there count as unread", () => {
  assert.match(fn("chatPaneShowing"), /return !isPhoneWidth\(window\.innerWidth\) \|\| narrowSplit\(\) \|\| phonePane\(\) !== "list";/);
  assert.match(fn("onMessageCreated"), /const focusedOnIt = viewing && appFocused\(\) && chatPaneShowing\(\);/);
  assert.match(fn("onDmReceived"), /const viewing = state\.view === "dm" && state\.currentDm && state\.currentDm\.user === dm\.user && chatPaneShowing\(\);/);
  assert.match(app, /window\.addEventListener\("focus", \(\) => \{\n    if \(!chatPaneShowing\(\)\) return;\n/);
});
