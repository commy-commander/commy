import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

test("no script declares the same top-level function twice", () => {
  const dir = join(import.meta.dirname, "..", "scripts");
  for (const file of readdirSync(dir).filter(f => f.endsWith(".js"))) {
    const seen = new Map();
    const dupes = [];
    for (const [, name] of readFileSync(join(dir, file), "utf8").matchAll(/^(?:async\s+)?function\s+([A-Za-z_$][\w$]*)\s*\(/gm)) {
      if (seen.has(name)) dupes.push(name);
      seen.set(name, true);
    }
    assert.deepEqual(dupes, [], `${file} declares twice: ${dupes.join(", ")}`);
  }
});
