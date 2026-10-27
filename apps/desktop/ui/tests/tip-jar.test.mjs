import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { TIP_JAR_HOST, TIP_JAR_MAX_PAGE, cleanTipJar, tipJarLabel } = require("../scripts/tip-jar.js");

const core = readFileSync(join(import.meta.dirname, "..", "..", "..", "..", "crates", "commy-core", "src", "tip_jar.rs"), "utf8");

test("the app and core agree on what a tip jar link is", () => {
  assert.match(core, new RegExp(`TIP_JAR_HOST: &str = "${TIP_JAR_HOST}";`));
  assert.match(core, new RegExp(`MAX_PAGE_CHARS: usize = ${TIP_JAR_MAX_PAGE};`));
  for (const input of ["rockets", " ko-fi.com/rockets ", "https://ko-fi.com/rockets/", "http://www.ko-fi.com/rockets", "https://KO-FI.com/rockets"]) {
    assert.equal(cleanTipJar(input), "https://ko-fi.com/rockets", input);
  }
  for (const input of ["", null, "https://evil.example/rockets", "https://ko-fi.com.evil.example/rockets", "https://ko-fi.com/rockets?x=1", "https://ko-fi.com/a/b", "https://ko-fi.com/", "javascript:alert(1)", "evil.example", "https://user@ko-fi.com/rockets", "ko-fi.com:8080/rockets", "a".repeat(TIP_JAR_MAX_PAGE + 1)]) {
    assert.equal(cleanTipJar(input), null, String(input));
  }
});

test("the link reads without its scheme", () => {
  assert.equal(tipJarLabel("https://ko-fi.com/rockets"), "ko-fi.com/rockets");
  assert.equal(tipJarLabel(null), "");
});
