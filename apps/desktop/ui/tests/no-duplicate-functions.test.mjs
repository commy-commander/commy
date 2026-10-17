import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const scriptsDir = join(import.meta.dirname, "..", "scripts");
const scripts = () => readdirSync(scriptsDir).filter(f => f.endsWith(".js"));

test("no script declares the same top-level function twice, since the later one silently replaces the earlier", () => {
  for (const file of scripts()) {
    const seen = new Map();
    const dupes = [];
    for (const [, name] of readFileSync(join(scriptsDir, file), "utf8").matchAll(/^(?:async\s+)?function\s+([A-Za-z_$][\w$]*)\s*\(/gm)) {
      if (seen.has(name)) dupes.push(name);
      seen.set(name, true);
    }
    assert.deepEqual(dupes, [], `${file} declares twice: ${dupes.join(", ")}`);
  }
});

test("no script declares isTauri, which Tauri's injected API already defines, so redeclaring it stops the whole script", () => {
  for (const file of scripts()) {
    const source = readFileSync(join(scriptsDir, file), "utf8");
    assert.doesNotMatch(source, /^\s*(?:const|let|var|function|class)\s+isTauri\b/m, `${file} declares isTauri`);
  }
});
