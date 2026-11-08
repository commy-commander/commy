#!/usr/bin/env bash
set -euo pipefail

usage="usage: ios-build.sh VERSION [--testflight]"
version="${1:?$usage}"
upload="${2:-}"
[[ -z "$upload" || "$upload" == --testflight ]] || { echo "$usage" >&2; exit 1; }
if [[ "$upload" == --testflight ]]; then
  : "${ASC_KEY_ID:?set ASC_KEY_ID to the App Store Connect API key ID}"
  : "${ASC_ISSUER_ID:?set ASC_ISSUER_ID to the App Store Connect issuer ID}"
fi

mobile="$(cd "$(dirname "$0")" && pwd)"
source "$mobile/release-env.sh" "$version"
build_number="$(date -u +%y%m%d.%H%M)"
echo "Build $build_number"

src_tauri="$(cd "$mobile/../src-tauri" && pwd)"
cd "$src_tauri"

[ -d gen/apple ] || {
  npx tauri ios init --ci
  npx tauri icon ../mobile/app-icon.png
}
"$mobile/ios-entitlements.sh"

config="$(printf '{"version":"%s","bundle":{"iOS":{"bundleVersion":"%s"}}}' "$release_app_version" "$build_number")"
rm -rf gen/apple/build/arm64
npx tauri ios build --ci --target aarch64 --export-method app-store-connect --config "$config"

ipa="$(find gen/apple/build/arm64 -name '*.ipa' | head -n 1)"
[ -n "$ipa" ] || { echo "no signed IPA was built" >&2; exit 1; }
out="$src_tauri/gen/apple/build/Commy-$version-$build_number.ipa"
cp "$ipa" "$out"
echo "Built $out"

if [[ "$upload" == --testflight ]]; then
  xcrun altool --upload-app --type ios --file "$out" \
    --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER_ID"
  echo "Sent to TestFlight"
fi
