import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

export const EXTENSION = "CommyNotifications";
export const SPEC = "notifications.yml";
export const SOURCES = "../../../mobile/ios/notifications";
export const SOUNDS = "../../../mobile/sounds";
export const KEYCHAIN_GROUP = "$(AppIdentifierPrefix)chat.commy.mobile.shared";

export function appTarget(project) {
  const match = project.match(/^\s*path:\s*["']?([^\s"'/]+)\/([^\s"'/]+)\.entitlements["']?\s*$/m);
  if (!match) throw new Error("no app entitlements file named in project.yml");
  return { dir: match[1], name: match[2] };
}

export function developmentTeam(project, env = process.env) {
  if (env.APPLE_DEVELOPMENT_TEAM) return env.APPLE_DEVELOPMENT_TEAM;
  const match = project.match(/^\s*DEVELOPMENT_TEAM:\s*["']?([A-Z0-9]{10})["']?\s*$/m);
  return match ? match[1] : null;
}

export function versionScript(appDir) {
  return [
    `app="$SRCROOT/${appDir}/Info.plist"`,
    `out="$TARGET_BUILD_DIR/$INFOPLIST_PATH"`,
    "for key in CFBundleShortVersionString CFBundleVersion; do",
    `  value="$(/usr/libexec/PlistBuddy -c "Print :$key" "$app" 2>/dev/null || true)"`,
    `  case "$value" in ""|*'$('*) continue ;; esac`,
    `  /usr/libexec/PlistBuddy -c "Set :$key $value" "$out" 2>/dev/null || /usr/libexec/PlistBuddy -c "Add :$key string $value" "$out"`,
    "done",
    "",
  ].join("\n");
}

export function notificationSpec({ app, bundleId, team }) {
  const settings = {
    PRODUCT_BUNDLE_IDENTIFIER: `${bundleId}.notifications`,
    SWIFT_VERSION: "5.0",
    TARGETED_DEVICE_FAMILY: "1,2",
    CODE_SIGN_STYLE: "Automatic",
    APPLICATION_EXTENSION_API_ONLY: "YES",
    SKIP_INSTALL: "YES",
  };
  if (team) settings.DEVELOPMENT_TEAM = team;
  return {
    targets: {
      [EXTENSION]: {
        type: "app-extension",
        platform: "iOS",
        sources: [{ path: SOURCES }],
        info: {
          path: `${EXTENSION}/Info.plist`,
          properties: {
            CFBundleDisplayName: "Commy",
            NSExtension: {
              NSExtensionPointIdentifier: "com.apple.usernotifications.service",
              NSExtensionPrincipalClass: "$(PRODUCT_MODULE_NAME).NotificationService",
            },
          },
        },
        entitlements: {
          path: `${EXTENSION}/${EXTENSION}.entitlements`,
          properties: { "keychain-access-groups": [KEYCHAIN_GROUP] },
        },
        settings: { base: settings },
        postBuildScripts: [{ name: "Match the app version", basedOnDependencyAnalysis: false, script: versionScript(app.dir) }],
      },
      [app.name]: {
        sources: [{ path: SOUNDS, buildPhase: "resources" }],
        dependencies: [{ target: EXTENSION, embed: true }],
      },
    },
  };
}

export function withInclude(project) {
  const entry = `  - ${SPEC}`;
  if (project.split("\n").includes(entry)) return project;
  if (/^include:\s*$/m.test(project)) return project.replace(/^include:\s*$/m, `include:\n${entry}`);
  if (/^include:/m.test(project)) throw new Error("project.yml has an include list this script does not understand");
  return `include:\n${entry}\n${project}`;
}

const here = path => fileURLToPath(new URL(path, import.meta.url));

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const apple = here("../src-tauri/gen/apple/");
  const projectPath = `${apple}project.yml`;
  const project = readFileSync(projectPath, "utf8");
  const bundleId = JSON.parse(readFileSync(here("../src-tauri/tauri.ios.conf.json"), "utf8")).identifier;
  if (!bundleId) throw new Error("tauri.ios.conf.json has no identifier");
  const spec = notificationSpec({ app: appTarget(project), bundleId, team: developmentTeam(project) });
  writeFileSync(`${apple}${SPEC}`, `${JSON.stringify(spec, null, 2)}\n`);
  writeFileSync(projectPath, withInclude(project));
  if (!process.argv.includes("--no-generate")) {
    const made = spawnSync("xcodegen", ["generate", "--spec", "project.yml"], { cwd: apple, stdio: "inherit" });
    if (made.status !== 0) throw new Error("xcodegen could not regenerate the Xcode project");
  }
  console.log(`Notification extension: ${EXTENSION} (${bundleId}.notifications)`);
}
