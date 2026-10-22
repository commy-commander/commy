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
  if (!out.includes('android:scheme="commy"')) {
    const main = out.indexOf('android:name=".MainActivity"');
    if (main < 0) throw new Error("AndroidManifest.xml has no MainActivity");
    out = insertBefore(out, "</activity>", [INVITE_LINKS], "            ", main);
  }
  return out;
}

export const GRADLE_DEPENDENCIES = [
  "com.google.firebase:firebase-messaging:24.1.0",
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

const here = path => fileURLToPath(new URL(path, import.meta.url));
const manifestPath = here("../src-tauri/gen/android/app/src/main/AndroidManifest.xml");
const sourcesFrom = here("./android/");
const sourcesTo = here("../src-tauri/gen/android/app/src/main/java/chat/commy/mobile/");
const gradlePath = here("../src-tauri/gen/android/app/build.gradle.kts");
const firebasePath = here("../src-tauri/gen/android/app/src/main/res/values/firebase.xml");

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const before = readFileSync(manifestPath, "utf8");
  const after = withCommyNeeds(before);
  if (after !== before) writeFileSync(manifestPath, after);
  mkdirSync(sourcesTo, { recursive: true });
  const sources = readdirSync(sourcesFrom).filter(name => name.endsWith(".kt"));
  for (const name of sources) copyFileSync(`${sourcesFrom}${name}`, `${sourcesTo}${name}`);
  const gradle = readFileSync(gradlePath, "utf8");
  const withDependencies = withCommyDependencies(gradle);
  if (withDependencies !== gradle) writeFileSync(gradlePath, withDependencies);
  console.log(after === before ? "AndroidManifest.xml already has what Commy needs" : "AndroidManifest.xml now asks for the microphone, camera, notifications, the call service, relaunching and invite links");
  console.log(`Copied ${sources.join(", ")} into the Android project`);
  const googleServices = readGoogleServices(process.env.GOOGLE_SERVICES_JSON);
  const packageName = applicationId(withDependencies);
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
