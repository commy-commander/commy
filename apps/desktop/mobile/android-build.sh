#!/usr/bin/env bash
set -euo pipefail

version="${1:?usage: android-build.sh VERSION}"
: "${ANDROID_UPLOAD_KEYSTORE_FILE:?set ANDROID_UPLOAD_KEYSTORE_FILE to the upload keystore}"
: "${ANDROID_UPLOAD_PASSWORD:?set ANDROID_UPLOAD_PASSWORD to the upload keystore password}"
export ANDROID_UPLOAD_KEYSTORE_FILE ANDROID_UPLOAD_PASSWORD

mobile="$(cd "$(dirname "$0")" && pwd)"
source "$mobile/release-env.sh" "$version"
version_code="$(( ($(date -u +%s) - 1767225600) / 60 ))"
echo "Version code $version_code"

src_tauri="$(cd "$mobile/../src-tauri" && pwd)"
cd "$src_tauri"

[ -d gen/android ] || npx tauri android init --ci
npx tauri icon ../mobile/app-icon.png
node ../mobile/android-project.mjs

config="$(printf '{"version":"%s","bundle":{"android":{"versionCode":%s}}}' "$release_app_version" "$version_code")"
outputs=gen/android/app/build/outputs
rm -rf "$outputs/bundle" "$outputs/apk"
npx tauri android build --ci --aab --apk --target aarch64 --target x86_64 --config "$config"

aab="$(find "$outputs/bundle" -name '*release*.aab' | head -n 1)"
apk="$(find "$outputs/apk" -name '*release*.apk' | head -n 1)"
[ -n "$aab" ] || { echo "no release AAB was built" >&2; exit 1; }
[ -n "$apk" ] || { echo "no release APK was built" >&2; exit 1; }
out="$src_tauri/gen/android/Commy-$version-$version_code"
cp "$aab" "$out.aab"
cp "$apk" "$out.apk"
echo "Built $out.aab for Google Play and $out.apk to install directly"
