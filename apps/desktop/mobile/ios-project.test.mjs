import test from "node:test";
import assert from "node:assert/strict";
import { appTarget, developmentTeam, notificationSpec, withInclude, versionScript, EXTENSION, SPEC, KEYCHAIN_GROUP } from "./ios-project.mjs";

const generated = `name: Commy
options:
  bundleIdPrefix: chat.commy
targets:
  Commy_iOS:
    type: application
    platform: iOS
    entitlements:
      path: Commy_iOS/Commy_iOS.entitlements
    settings:
      base:
        DEVELOPMENT_TEAM: ABCDE12345
`;

test("finds the app target from its entitlements file", () => {
  assert.deepEqual(appTarget(generated), { dir: "Commy_iOS", name: "Commy_iOS" });
  assert.throws(() => appTarget("name: Commy\n"), /entitlements/);
});

test("the team comes from the environment first, then the generated project", () => {
  assert.equal(developmentTeam(generated, {}), "ABCDE12345");
  assert.equal(developmentTeam(generated, { APPLE_DEVELOPMENT_TEAM: "ZZZZZ99999" }), "ZZZZZ99999");
  assert.equal(developmentTeam("name: Commy\n", {}), null);
});

test("the extension is embedded in the app and shares one keychain group with it", () => {
  const spec = notificationSpec({ app: appTarget(generated), bundleId: "chat.commy.mobile", team: "ABCDE12345" });
  const extension = spec.targets[EXTENSION];
  assert.equal(extension.type, "app-extension");
  assert.equal(extension.settings.base.PRODUCT_BUNDLE_IDENTIFIER, "chat.commy.mobile.notifications");
  assert.equal(extension.settings.base.DEVELOPMENT_TEAM, "ABCDE12345");
  assert.equal(extension.info.properties.NSExtension.NSExtensionPointIdentifier, "com.apple.usernotifications.service");
  assert.deepEqual(extension.entitlements.properties["keychain-access-groups"], [KEYCHAIN_GROUP]);
  assert.deepEqual(spec.targets.Commy_iOS.dependencies, [{ target: EXTENSION, embed: true }]);
  const unsigned = notificationSpec({ app: appTarget(generated), bundleId: "chat.commy.mobile", team: null });
  assert.equal("DEVELOPMENT_TEAM" in unsigned.targets[EXTENSION].settings.base, false);
});

test("the extension takes the app's version from the app's Info.plist", () => {
  const script = versionScript("Commy_iOS");
  assert.match(script, /\$SRCROOT\/Commy_iOS\/Info\.plist/);
  assert.match(script, /CFBundleShortVersionString CFBundleVersion/);
});

test("the app entitlements share the same keychain group", async () => {
  const { readFileSync } = await import("node:fs");
  const plist = readFileSync(new URL("../src-tauri/Entitlements.ios.plist", import.meta.url), "utf8");
  assert.ok(plist.includes(`<string>${KEYCHAIN_GROUP}</string>`));
});

test("the spec is included once, whatever was there before", () => {
  const once = withInclude(generated);
  assert.ok(once.startsWith(`include:\n  - ${SPEC}\n`));
  assert.equal(withInclude(once), once);
  const listed = withInclude(`include:\n  - other.yml\n${generated}`);
  assert.ok(listed.startsWith(`include:\n  - ${SPEC}\n  - other.yml\n`));
  assert.throws(() => withInclude(`include: [other.yml]\n${generated}`), /include/);
});
