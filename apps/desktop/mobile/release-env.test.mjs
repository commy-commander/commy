import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

const releaseEnv = fileURLToPath(new URL("./release-env.sh", import.meta.url)).replace(/\\/g, "/");
const repo = new URL("../../../", import.meta.url);
const TAURI_MOBILE_BUILDS = [
  { source: "apps/desktop/src-tauri/src", buildScript: "apps/desktop/src-tauri/build.rs" },
  { source: "crates/commy-net/src", buildScript: "crates/commy-net/build.rs" },
];

function sourced(version, script) {
  return execFileSync("bash", ["-c", `source "${releaseEnv}" "$1" >/dev/null && ${script}`, "release-env", version], { encoding: "utf8" });
}

function checkBuilt(version, contents) {
  return spawnSync("bash", ["-c", `source "${releaseEnv}" "$1" >/dev/null && printf '%s' "$2" | expect_built_servers "The app"`, "release-env", version, contents], { encoding: "utf8" });
}

function rustFiles(dir) {
  return readdirSync(new URL(dir + "/", repo), { withFileTypes: true, recursive: true })
    .filter(entry => entry.isFile() && entry.name.endsWith(".rs"))
    .map(entry => readFileSync(`${entry.parentPath ?? entry.path}/${entry.name}`, "utf8"));
}

test("a release candidate hands its staging servers to the mobile build under the TAURI_ prefix Tauri passes through", () => {
  const out = sourced("1.0.0-rc.8", 'printf "%s|%s|%s|%s" "$TAURI_COMMY_DEFAULT_ACCOUNT_SERVER" "$TAURI_COMMY_DEFAULT_RELAY" "$TAURI_COMMY_TEST_BUILD" "$TAURI_COMMY_VERSION"');
  assert.equal(out, "https://staging.commy.chat|https://relay-staging.commy.chat:8443|1|1.0.0-rc.8");
});

test("a final release keeps the production defaults", () => {
  const out = sourced("1.0.0", 'printf "%s|%s|%s" "${TAURI_COMMY_DEFAULT_ACCOUNT_SERVER:-}" "${COMMY_TEST_BUILD:-}" "$TAURI_COMMY_VERSION"');
  assert.equal(out, "||1.0.0");
});

test("a release candidate builds as the Commy Beta iPhone app with its own icon, name and link scheme", () => {
  const out = sourced("1.0.1-rc.1", 'printf "%s|%s|%s|%s" "$COMMY_IOS_BUNDLE_ID" "$release_app_name" "$(basename "$release_app_icon")" "$(ios_build_config 261122.2300)"');
  const [bundleId, name, icon, config] = out.split("|");
  assert.equal(bundleId, "chat.commy.mobile.beta");
  assert.equal(name, "Commy Beta");
  assert.equal(icon, "app-icon-beta.png");
  const parsed = JSON.parse(config);
  assert.equal(parsed.identifier, "chat.commy.mobile.beta");
  assert.equal(parsed.version, "1.0.1");
  assert.equal(parsed.bundle.iOS.bundleVersion, "261122.2300");
  assert.match(parsed.bundle.iOS.infoPlist, /\/mobile\/ios\/beta\/Info\.plist$/);
  const plist = readFileSync(parsed.bundle.iOS.infoPlist, "utf8");
  assert.match(plist, /<key>CFBundleDisplayName<\/key>\s*<string>Commy Beta<\/string>/);
  assert.match(plist, /<string>commy-beta<\/string>/);
  assert.doesNotMatch(plist, /<string>commy<\/string>/);
  assert.ok(readFileSync(new URL("./app-icon-beta.png", import.meta.url)).length > 0);
});

test("a final release builds the store iPhone app", () => {
  const out = sourced("1.0.1", 'printf "%s|%s|%s|%s" "$COMMY_IOS_BUNDLE_ID" "$release_app_name" "$(basename "$release_app_icon")" "$(ios_build_config)"');
  const [bundleId, name, icon, config] = out.split("|");
  assert.equal(bundleId, "chat.commy.mobile");
  assert.equal(name, "Commy");
  assert.equal(icon, "app-icon.png");
  assert.deepEqual(JSON.parse(config), { identifier: "chat.commy.mobile", version: "1.0.1", bundle: { iOS: {} } });
});

test("a release candidate that was built without the staging server fails the check", () => {
  assert.notEqual(checkBuilt("1.0.0-rc.8", "https://commy.chat").status, 0);
  assert.equal(checkBuilt("1.0.0-rc.8", "xx https://staging.commy.chat xx").status, 0);
});

test("a final release that points at the staging server fails the check", () => {
  assert.notEqual(checkBuilt("1.0.0", "https://staging.commy.chat").status, 0);
  assert.equal(checkBuilt("1.0.0", "https://commy.chat").status, 0);
});

test("every build-time COMMY_ value a mobile app reads is forwarded by its crate's build script", () => {
  for (const { source, buildScript } of TAURI_MOBILE_BUILDS) {
    const script = readFileSync(new URL(buildScript, repo), "utf8");
    const names = new Set(rustFiles(source).flatMap(text => [...text.matchAll(/option_env!\("(COMMY_[A-Z_]+)"\)/g)].map(match => match[1])));
    assert.ok(names.size > 0, `${source} reads build-time values`);
    for (const name of names) assert.ok(script.includes(`"${name}"`), `${buildScript} forwards ${name}`);
  }
});
