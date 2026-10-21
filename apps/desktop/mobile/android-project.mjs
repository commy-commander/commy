import { copyFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
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
];

export const FEATURES = [
  "android.hardware.camera",
  "android.hardware.microphone",
];

export const SERVICES = [
  { name: ".CallService", types: "microphone|mediaPlayback" },
];

const usesPermission = name => `<uses-permission android:name="${name}" />`;
const usesFeature = name => `<uses-feature android:name="${name}" android:required="false" />`;
const service = ({ name, types }) =>
  `<service android:name="${name}" android:exported="false" android:foregroundServiceType="${types}" />`;

function insertBefore(xml, marker, lines, indent) {
  const anchor = xml.indexOf(marker);
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
  const services = SERVICES.filter(s => !out.includes(`"${s.name}"`)).map(service);
  if (services.length) out = insertBefore(out, "</application>", services, "        ");
  return out;
}

const here = path => fileURLToPath(new URL(path, import.meta.url));
const manifestPath = here("../src-tauri/gen/android/app/src/main/AndroidManifest.xml");
const sourcesFrom = here("./android/");
const sourcesTo = here("../src-tauri/gen/android/app/src/main/java/chat/commy/mobile/");

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const before = readFileSync(manifestPath, "utf8");
  const after = withCommyNeeds(before);
  if (after !== before) writeFileSync(manifestPath, after);
  mkdirSync(sourcesTo, { recursive: true });
  const sources = readdirSync(sourcesFrom).filter(name => name.endsWith(".kt"));
  for (const name of sources) copyFileSync(`${sourcesFrom}${name}`, `${sourcesTo}${name}`);
  console.log(after === before ? "AndroidManifest.xml already has what Commy needs" : "AndroidManifest.xml now asks for the microphone, camera, notifications and the call service");
  console.log(`Copied ${sources.join(", ")} into the Android project`);
}
