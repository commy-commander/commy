import { copyFileSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

export const PERMISSIONS = [
  "android.permission.INTERNET",
  "android.permission.ACCESS_NETWORK_STATE",
  "android.permission.RECORD_AUDIO",
  "android.permission.MODIFY_AUDIO_SETTINGS",
  "android.permission.CAMERA",
  "android.permission.POST_NOTIFICATIONS",
  "android.permission.FOREGROUND_SERVICE",
  "android.permission.FOREGROUND_SERVICE_MICROPHONE",
  "android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK",
  "android.permission.FOREGROUND_SERVICE_MEDIA_PROJECTION",
  "android.permission.WAKE_LOCK",
  "android.permission.BLUETOOTH",
];

export const MAX_SDK = { "android.permission.BLUETOOTH": 30 };

export const FEATURES = [
  "android.hardware.camera",
  "android.hardware.microphone",
];

export const COMPONENTS = [
  {
    name: ".CallService",
    xml: `<service android:name=".CallService" android:exported="false" android:foregroundServiceType="microphone|mediaPlayback" />`,
  },
  {
    name: ".ScreenShareService",
    xml: `<service android:name=".ScreenShareService" android:exported="false" android:foregroundServiceType="mediaProjection" />`,
  },
  {
    name: ".WakeService",
    xml: `<service android:name=".WakeService" android:exported="false"><intent-filter><action android:name="com.google.firebase.MESSAGING_EVENT" /></intent-filter></service>`,
  },
  {
    name: ".RelaunchActivity",
    xml: `<activity android:name=".RelaunchActivity" android:process=":relaunch" android:exported="false" android:theme="@android:style/Theme.Translucent.NoTitleBar" />`,
  },
];

const usesPermission = name => (MAX_SDK[name] ? `<uses-permission android:name="${name}" android:maxSdkVersion="${MAX_SDK[name]}" />` : `<uses-permission android:name="${name}" />`);
const usesFeature = name => `<uses-feature android:name="${name}" android:required="false" />`;

export const INVITE_LINKS = `<intent-filter><action android:name="android.intent.action.VIEW" /><category android:name="android.intent.category.DEFAULT" /><category android:name="android.intent.category.BROWSABLE" /><data android:scheme="commy" android:host="i" /></intent-filter>`;
export const SIGN_IN_RETURN = `<intent-filter><action android:name="android.intent.action.VIEW" /><category android:name="android.intent.category.DEFAULT" /><category android:name="android.intent.category.BROWSABLE" /><data android:scheme="commy" android:host="signed-in" /></intent-filter>`;
const LINK_FILTERS = [
  { marker: 'android:host="i"', xml: INVITE_LINKS },
  { marker: 'android:host="signed-in"', xml: SIGN_IN_RETURN },
];

function insertBefore(xml, marker, lines, indent, from = 0) {
  const anchor = xml.indexOf(marker, from);
  if (anchor < 0) throw new Error(`AndroidManifest.xml has no ${marker} element`);
  const lineStart = xml.lastIndexOf("\n", anchor) + 1;
  const block = lines.map(line => (line ? `${indent}${line}\n` : "\n")).join("");
  return `${xml.slice(0, lineStart)}${block}${xml.slice(lineStart)}`;
}

export function withCommyNeeds(xml) {
  const missing = [
    ...PERMISSIONS.filter(name => !xml.includes(`"${name}"`)).map(usesPermission),
    ...FEATURES.filter(name => !xml.includes(`"${name}"`)).map(usesFeature),
  ];
  let out = xml;
  if (missing.length) out = insertBefore(out, "<application", [...missing, ""], "    ");
  const components = COMPONENTS.filter(c => !out.includes(`"${c.name}"`)).map(c => c.xml);
  if (components.length) out = insertBefore(out, "</application>", components, "        ");
  const filters = LINK_FILTERS.filter(f => !out.includes(f.marker)).map(f => f.xml);
  if (filters.length) {
    const main = out.indexOf('android:name=".MainActivity"');
    if (main < 0) throw new Error("AndroidManifest.xml has no MainActivity");
    out = insertBefore(out, "</activity>", filters, "            ", main);
  }
  return out;
}

export const GRADLE_DEPENDENCIES = [
  "com.google.firebase:firebase-messaging:24.1.0",
  "androidx.browser:browser:1.8.0",
  "androidx.credentials:credentials:1.3.0",
  "androidx.credentials:credentials-play-services-auth:1.3.0",
];

export function withCommyDependencies(gradle) {
  const block = gradle.match(/^dependencies \{[ \t]*\n/m);
  if (!block) throw new Error("app/build.gradle.kts has no dependencies block");
  const missing = GRADLE_DEPENDENCIES.filter(coordinate => !gradle.includes(`"${coordinate}"`));
  if (!missing.length) return gradle;
  const at = block.index + block[0].length;
  const lines = missing.map(coordinate => `    implementation("${coordinate}")\n`).join("");
  return `${gradle.slice(0, at)}${lines}${gradle.slice(at)}`;
}

export const DEBUG_KEY = { storePassword: "android", keyAlias: "androiddebugkey", keyPassword: "android" };

const kotlinString = text => `"${String(text).replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\$/g, "\\$")}"`;

export const UPLOAD_KEY = { alias: "commy-upload", passwordEnv: "ANDROID_UPLOAD_PASSWORD" };

const signingBlock = (name, storeFile, password, alias) => [
  `${name} {`,
  `    storeFile = file(${kotlinString(storeFile)})`,
  `    storePassword = ${password}`,
  `    keyAlias = ${kotlinString(alias)}`,
  `    keyPassword = ${password}`,
  `}`,
];

function signingConfigs({ debugStore, releaseStore }) {
  const debug = debugStore ? signingBlock(`getByName("debug")`, debugStore, kotlinString(DEBUG_KEY.storePassword), DEBUG_KEY.keyAlias) : [];
  const release = releaseStore ? signingBlock(`create("release")`, releaseStore, `System.getenv(${kotlinString(UPLOAD_KEY.passwordEnv)})`, UPLOAD_KEY.alias) : [];
  return ["signingConfigs {", ...[...debug, ...release].map(line => `    ${line}`), "}"];
}

export function withSigning(gradle, { debugStore, releaseStore } = {}) {
  if (!debugStore && !releaseStore) return gradle;
  if (gradle.includes("signingConfigs {")) return gradle;
  const block = gradle.match(/^([ \t]*)buildTypes \{/m);
  if (!block) throw new Error("app/build.gradle.kts has no buildTypes block");
  const indent = block[1];
  const lines = signingConfigs({ debugStore, releaseStore }).map(line => `${indent}${line}`).join("\n");
  let out = `${gradle.slice(0, block.index)}${lines}\n${gradle.slice(block.index)}`;
  if (releaseStore) {
    const release = out.match(/^([ \t]*)getByName\("release"\) \{\n/m);
    if (!release) throw new Error("app/build.gradle.kts has no release build type");
    const at = release.index + release[0].length;
    out = `${out.slice(0, at)}${release[1]}    signingConfig = signingConfigs.getByName("release")\n${out.slice(at)}`;
  }
  return out;
}

export const withDebugSigning = (gradle, storeFile) => withSigning(gradle, { debugStore: storeFile });

export function withNativeSymbols(gradle) {
  if (gradle.includes("debugSymbolLevel")) return gradle;
  const release = gradle.match(/^([ \t]*)getByName\("release"\) \{\n/m);
  if (!release) return gradle;
  const at = release.index + release[0].length;
  const indent = `${release[1]}    `;
  const out = `${gradle.slice(0, at)}${indent}ndk {\n${indent}    debugSymbolLevel = "SYMBOL_TABLE"\n${indent}}\n${gradle.slice(at)}`;
  const android = out.match(/^([ \t]*)android \{\n/m);
  if (!android || out.includes("ndkPath")) return out;
  const from = android.index + android[0].length;
  return `${out.slice(0, from)}${android[1]}    System.getenv("NDK_HOME")?.takeIf { it.isNotBlank() }?.let { ndkPath = it }\n${out.slice(from)}`;
}

export function applicationId(gradle) {
  return gradle.match(/applicationId\s*=\s*"([^"]+)"/)?.[1] ?? null;
}

const FIREBASE_STRINGS = ["google_app_id", "gcm_defaultSenderId", "google_api_key", "project_id"];
const escapeXml = text => String(text).replace(/\\/g, "\\\\").replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "\\'");

export function firebasePackages(googleServices) {
  const config = typeof googleServices === "string" ? JSON.parse(googleServices) : googleServices;
  return (config.client ?? []).map(c => c.client_info?.android_client_info?.package_name).filter(Boolean);
}

export function firebaseValues(googleServices, packageName) {
  const config = typeof googleServices === "string" ? JSON.parse(googleServices) : googleServices;
  const client = (config.client ?? []).find(c => c.client_info?.android_client_info?.package_name === packageName);
  if (!client) return null;
  const values = {
    google_app_id: client.client_info?.mobilesdk_app_id,
    gcm_defaultSenderId: config.project_info?.project_number,
    google_api_key: client.api_key?.[0]?.current_key,
    project_id: config.project_info?.project_id,
  };
  if (FIREBASE_STRINGS.some(name => !values[name])) return null;
  const keep = FIREBASE_STRINGS.map(name => `@string/${name}`).join(",");
  const strings = FIREBASE_STRINGS.map(name => `    <string name="${name}" translatable="false">${escapeXml(values[name])}</string>\n`).join("");
  return `<?xml version="1.0" encoding="utf-8"?>\n<resources xmlns:tools="http://schemas.android.com/tools" tools:keep="${keep}">\n${strings}</resources>\n`;
}

export function readGoogleServices(value, readFile = path => readFileSync(path, "utf8")) {
  const text = value?.trim();
  if (!text) return null;
  if (text.startsWith("{")) return text;
  if (/^[A-Za-z0-9+/=\s]+$/.test(text)) {
    const decoded = Buffer.from(text, "base64").toString("utf8").trim();
    if (decoded.startsWith("{")) return decoded;
  }
  return readFile(text);
}

export const LAUNCHER_LAYERS = { background: "commy_launcher_background", foreground: "commy_launcher_foreground", monochrome: "commy_launcher_monochrome" };
export const LAUNCHER_NAMES = ["ic_launcher", "ic_launcher_round"];
export const NOTIFICATION_ICON = "commy_notification";
export const NOTIFICATION_SOUNDS = ["commy_message.wav", "commy_dm.wav"];

export function launcherIcon(layers = LAUNCHER_LAYERS) {
  const lines = Object.entries(layers).map(([layer, name]) => `    <${layer} android:drawable="@drawable/${name}" />\n`).join("");
  return `<?xml version="1.0" encoding="utf-8"?>\n<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n${lines}</adaptive-icon>\n`;
}

const here = path => fileURLToPath(new URL(path, import.meta.url));
const manifestPath = here("../src-tauri/gen/android/app/src/main/AndroidManifest.xml");
const sourcesFrom = here("./android/");
const sourcesTo = here("../src-tauri/gen/android/app/src/main/java/chat/commy/mobile/");
const gradlePath = here("../src-tauri/gen/android/app/build.gradle.kts");
const firebasePath = here("../src-tauri/gen/android/app/src/main/res/values/firebase.xml");
const keepRulesFrom = here("./android/commy-rules.pro");
const keepRulesTo = here("../src-tauri/gen/android/app/commy-rules.pro");
const launcherFrom = here("./android-icon/");
const launcherLayersTo = here("../src-tauri/gen/android/app/src/main/res/drawable-nodpi/");
const launcherIconTo = here("../src-tauri/gen/android/app/src/main/res/mipmap-anydpi-v26/");
const soundsFrom = here("./sounds/");
const soundsTo = here("../src-tauri/gen/android/app/src/main/res/raw/");

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const before = readFileSync(manifestPath, "utf8");
  const after = withCommyNeeds(before);
  if (after !== before) writeFileSync(manifestPath, after);
  mkdirSync(sourcesTo, { recursive: true });
  const sources = readdirSync(sourcesFrom).filter(name => name.endsWith(".kt"));
  for (const name of sources) copyFileSync(`${sourcesFrom}${name}`, `${sourcesTo}${name}`);
  copyFileSync(keepRulesFrom, keepRulesTo);
  mkdirSync(launcherLayersTo, { recursive: true });
  mkdirSync(launcherIconTo, { recursive: true });
  for (const name of [...Object.values(LAUNCHER_LAYERS), NOTIFICATION_ICON]) copyFileSync(`${launcherFrom}${name}.png`, `${launcherLayersTo}${name}.png`);
  for (const name of LAUNCHER_NAMES) writeFileSync(`${launcherIconTo}${name}.xml`, launcherIcon());
  mkdirSync(soundsTo, { recursive: true });
  for (const name of NOTIFICATION_SOUNDS) copyFileSync(`${soundsFrom}${name}`, `${soundsTo}${name}`);
  const gradle = readFileSync(gradlePath, "utf8");
  const debugStore = process.env.ANDROID_DEBUG_KEYSTORE_FILE;
  const releaseStore = process.env.ANDROID_UPLOAD_KEYSTORE_FILE;
  const withDependencies = withNativeSymbols(withSigning(withCommyDependencies(gradle), { debugStore, releaseStore }));
  if (withDependencies !== gradle) writeFileSync(gradlePath, withDependencies);
  console.log(debugStore ? "Debug builds sign with the kept debug key" : "ANDROID_DEBUG_KEYSTORE_FILE is not set, so debug builds sign with a new key");
  console.log(releaseStore ? `Release builds sign with the upload key, password from ${UPLOAD_KEY.passwordEnv}` : "ANDROID_UPLOAD_KEYSTORE_FILE is not set, so release builds are unsigned");
  console.log(after === before ? "AndroidManifest.xml already has what Commy needs" : "AndroidManifest.xml now asks for the microphone, camera, notifications, the call and screen share services, relaunching, invite links and the sign-in return link");
  console.log(`Copied ${sources.join(", ")} into the Android project`);
  console.log("The launcher icon is the star on night blue, with a one-colour layer for themed icons, and notifications show the white star");
  console.log(`Notifications play Commy's own sounds: ${NOTIFICATION_SOUNDS.join(", ")}`);
  const googleServices = readGoogleServices(process.env.GOOGLE_SERVICES_JSON);
  const packageName = applicationId(gradle);
  const firebase = googleServices && firebaseValues(googleServices, packageName);
  if (firebase) {
    writeFileSync(firebasePath, firebase);
    console.log(`Push wake-ups are on: wrote the Firebase settings for ${packageName}`);
  } else {
    rmSync(firebasePath, { force: true });
    if (!googleServices) console.log("GOOGLE_SERVICES_JSON is not set, so push wake-ups are off");
    else {
      const found = firebasePackages(googleServices);
      console.log(`GOOGLE_SERVICES_JSON has Android apps for ${found.join(", ") || "no package"}, but this build is ${packageName ?? "an unknown package"}, so push wake-ups are off`);
    }
  }
}
