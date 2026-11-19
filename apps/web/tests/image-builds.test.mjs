import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const REPO = new URL("../../../", import.meta.url);
const IMAGES_THAT_COMPILE_RUST = ["auth/Dockerfile", "crates/commy-sync-server/Dockerfile"];

function runInstructions(dockerfile) {
  return dockerfile.replace(/\\\r?\n/g, " ").split(/\r?\n/).filter(line => line.startsWith("RUN "));
}

function readImage(path) {
  return readFileSync(new URL(path, REPO), "utf8");
}

for (const path of IMAGES_THAT_COMPILE_RUST) {
  test(`${path} compiles the checked-out source, never a build left in the shared target cache`, () => {
    const cached = runInstructions(readImage(path)).filter(run => run.includes("target=/src/target"));
    assert.ok(cached.length > 0, "the image keeps a target cache");
    for (const run of cached) {
      assert.match(run, /target=\/src\/target,sharing=locked/, "one build at a time uses the target cache");
      const touch = run.indexOf("-exec touch {} +");
      assert.ok(touch > 0, "every source file is touched before the build");
      assert.match(run, /find \/src -path \/src\/target -prune/, "the target cache itself is left alone");
      assert.ok(touch < Math.max(run.indexOf("cargo build"), run.indexOf("node apps/web/build.mjs")), "the touch runs before the build");
    }
  });
}

test("the run instruction reader joins continued lines", () => {
  const runs = runInstructions("FROM x\nRUN a \\\n    && b\nCOPY . .\n");
  assert.deepEqual(runs, ["RUN a      && b"]);
});
