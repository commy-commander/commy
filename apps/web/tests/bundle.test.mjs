import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { Script } from "node:vm";
import { ASSETS_DIR, bundleWebPage, contentHash, hashedName, joinScripts, rewriteRelativeImports, scriptIsStrict, strictnessRuns, stylesheetGroups } from "../bundle.mjs";
import { injectWebShell } from "../shell.mjs";

const repo = join(import.meta.dirname, "..", "..", "..");
const WEB_ONLY = {
  "scripts/web-commands.js": "apps/web/ui/web-commands.js",
  "scripts/web-voice.js": "apps/web/ui/web-voice.js",
  "scripts/web-speech.js": "apps/web/ui/web-speech.js",
  "scripts/web-bridge.js": "apps/web/ui/web-bridge.js",
  "styles/web.css": "apps/web/ui/web.css",
};
const FAKE_PACKAGE = {
  "scripts/pkg/commy_web_client.js": "export default async function init(input) { input ??= new URL('commy_web_client_bg.wasm', import.meta.url); }\n",
  "scripts/pkg/commy_web_client_bg.wasm": "\0asm",
};

function memoryDist(files) {
  const written = new Map();
  const removed = new Set();
  return {
    written,
    removed,
    read(path) {
      if (written.has(path)) return written.get(path);
      if (path in files) return Buffer.from(files[path]);
      throw new Error(`no file ${path}`);
    },
    write: (path, bytes) => written.set(path, Buffer.from(bytes)),
    remove: path => removed.add(path),
  };
}

function repoDist() {
  const dist = memoryDist(FAKE_PACKAGE);
  const read = dist.read;
  dist.read = path => {
    if (path in FAKE_PACKAGE || dist.written.has(path)) return read(path);
    return readFileSync(join(repo, WEB_ONLY[path] ?? join("apps", "desktop", "ui", path)));
  };
  return dist;
}

const realPage = () => injectWebShell(readFileSync(join(repo, "apps", "desktop", "ui", "index.html"), "utf8"), { relays: ["https://relay.commy.chat:8443"], base: "/web/" });
const wasmPackage = { js: "scripts/pkg/commy_web_client.js", wasm: "scripts/pkg/commy_web_client_bg.wasm" };

test("a script is strict only when it opens with the use strict directive", () => {
  assert.equal(scriptIsStrict('"use strict";\nconst a = 1;'), true);
  assert.equal(scriptIsStrict("// note\n/* more */\n'use strict';"), true);
  assert.equal(scriptIsStrict('const a = 1;\n"use strict";'), false);
  assert.equal(scriptIsStrict("function f() {\n  \"use strict\";\n}"), false);
});

test("strict and sloppy scripts stay in separate files so each keeps its own mode", () => {
  const runs = strictnessRuns([
    { src: "a.js", code: '"use strict";' },
    { src: "b.js", code: '"use strict";' },
    { src: "c.js", code: "var c;" },
    { src: "d.js", code: '"use strict";' },
  ]);
  assert.deepEqual(runs.map(run => [run.strict, run.scripts.map(s => s.src)]), [
    [true, ["a.js", "b.js"]],
    [false, ["c.js"]],
    [true, ["d.js"]],
  ]);
});

test("joined scripts cannot run into each other", () => {
  const joined = joinScripts(["const a = 1 // trailing comment", "(function () {})()"]);
  assert.doesNotThrow(() => new Script(joined));
});

test("hashed names change with the content and live in the assets folder", () => {
  assert.equal(hashedName("styles/styles.css", "a"), `${ASSETS_DIR}/styles-${contentHash("a")}.css`);
  assert.notEqual(hashedName("styles/styles.css", "a"), hashedName("styles/styles.css", "b"));
});

test("stylesheets that switch together share one file and keep the attribute the switch looks for", () => {
  const html = [
    '<link rel="stylesheet" href="styles/a.css" />',
    '<link rel="stylesheet" href="styles/b.css" />',
    '<link rel="stylesheet" id="studio-one" href="styles/c.css" media="not all" />',
    '<link rel="stylesheet" id="studio-two" href="styles/d.css" media="not all" />',
  ].join("\n  ");
  const groups = stylesheetGroups(html);
  assert.deepEqual(groups.map(group => [group.id, group.media, group.links.map(link => link.href)]), [
    [null, null, ["styles/a.css", "styles/b.css"]],
    ["studio-one", "not all", ["styles/c.css", "styles/d.css"]],
  ]);
});

test("a relative import in a bundled script points at its hashed copy", () => {
  const moved = new Map([["scripts/pkg/client.js", "assets/client-abc.js"]]);
  assert.equal(rewriteRelativeImports('import("./pkg/client.js")', "scripts/bridge.js", moved), 'import("./client-abc.js")');
  assert.throws(() => rewriteRelativeImports('import("./pkg/other.js")', "scripts/bridge.js", moved), /does not hash/);
});

test("the web page loads a handful of hashed files instead of every script and stylesheet", () => {
  const dist = repoDist();
  const html = bundleWebPage(realPage(), { ...dist, wasmPackage });
  const scripts = [...html.matchAll(/<script src="([^"]+)"><\/script>/g)].map(m => m[1]);
  const sheets = [...html.matchAll(/<link rel="stylesheet"[^>]*href="([^"]+)"/g)].map(m => m[1]);
  assert.ok(scripts.length > 0 && scripts.length <= 20, `${scripts.length} scripts`);
  assert.equal(sheets.length, 2);
  for (const src of [...scripts, ...sheets]) assert.match(src, /^assets\/[a-z-]+-[0-9a-f]{16}\.(?:js|css)$/);
  assert.match(html, /<link rel="stylesheet" id="studio-[a-z-]+" href="assets\/[^"]+" media="not all" \/>/);
  assert.ok(html.indexOf("Content-Security-Policy") < html.indexOf('rel="modulepreload"'));
  assert.deepEqual([...dist.removed].sort(), Object.keys(FAKE_PACKAGE).sort());
});

test("every bundled script still compiles, so no two scripts declare the same name", () => {
  const dist = repoDist();
  const html = bundleWebPage(realPage(), { ...dist, wasmPackage });
  for (const [, src] of html.matchAll(/<script src="([^"]+)"><\/script>/g)) {
    assert.doesNotThrow(() => new Script(dist.written.get(src).toString("utf8"), { filename: src }), src);
  }
});

test("the bridge imports the hashed client, and the client loads the hashed wasm", () => {
  const dist = repoDist();
  const html = bundleWebPage(realPage(), { ...dist, wasmPackage });
  const [, js] = /<link rel="modulepreload" href="([^"]+)"/.exec(html);
  const [, wasm] = /<link rel="preload" href="([^"]+)" as="fetch"/.exec(html);
  const bundled = [...html.matchAll(/<script src="([^"]+)"><\/script>/g)].map(([, src]) => dist.written.get(src).toString("utf8")).join("");
  assert.ok(bundled.includes(`import("./${js.slice(ASSETS_DIR.length + 1)}")`));
  assert.ok(dist.written.get(js).toString("utf8").includes(`'${wasm.slice(ASSETS_DIR.length + 1)}'`));
});

test("fonts named by the stylesheets get hashed copies next to the bundle", () => {
  const dist = repoDist();
  bundleWebPage(realPage(), { ...dist, wasmPackage });
  const css = [...dist.written].filter(([path]) => path.endsWith(".css")).map(([, bytes]) => bytes.toString("utf8")).join("");
  assert.doesNotMatch(css, /url\(["']?\.\.\//);
  assert.ok([...dist.written.keys()].some(path => /^assets\/figtree-latin-[0-9a-f]{16}\.woff2$/.test(path)));
});
