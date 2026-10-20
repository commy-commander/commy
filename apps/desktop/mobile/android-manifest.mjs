import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

export const PERMISSIONS = [
  "android.permission.INTERNET",
  "android.permission.ACCESS_NETWORK_STATE",
  "android.permission.RECORD_AUDIO",
  "android.permission.MODIFY_AUDIO_SETTINGS",
  "android.permission.CAMERA",
  "android.permission.POST_NOTIFICATIONS",
];

export const FEATURES = [
  "android.hardware.camera",
  "android.hardware.microphone",
];

const usesPermission = name => `<uses-permission android:name="${name}" />`;
const usesFeature = name => `<uses-feature android:name="${name}" android:required="false" />`;

export function withCommyNeeds(xml) {
  const missing = [
    ...PERMISSIONS.filter(name => !xml.includes(`"${name}"`)).map(usesPermission),
    ...FEATURES.filter(name => !xml.includes(`"${name}"`)).map(usesFeature),
  ];
  if (!missing.length) return xml;
  const anchor = xml.indexOf("<application");
  if (anchor < 0) throw new Error("AndroidManifest.xml has no <application> element");
  const lineStart = xml.lastIndexOf("\n", anchor) + 1;
  const lines = missing.map(line => `    ${line}\n`).join("");
  return `${xml.slice(0, lineStart)}${lines}\n${xml.slice(lineStart)}`;
}

const manifestPath = fileURLToPath(new URL("../src-tauri/gen/android/app/src/main/AndroidManifest.xml", import.meta.url));

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const before = readFileSync(manifestPath, "utf8");
  const after = withCommyNeeds(before);
  if (after !== before) writeFileSync(manifestPath, after);
  console.log(after === before ? "AndroidManifest.xml already has what Commy needs" : "AndroidManifest.xml now asks for the microphone, camera and notifications");
}
