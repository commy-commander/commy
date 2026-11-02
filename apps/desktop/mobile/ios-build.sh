#!/usr/bin/env bash
set -euo pipefail

usage="usage: ios-build.sh VERSION [--testflight]"
version="${1:?$usage}"
upload="${2:-}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] \
  || { echo "VERSION must look like 0.1.16 or 0.1.16-rc.1" >&2; exit 1; }
[[ -z "$upload" || "$upload" == --testflight ]] || { echo "$usage" >&2; exit 1; }
if [[ "$upload" == --testflight ]]; then
  : "${ASC_KEY_ID:?set ASC_KEY_ID to the App Store Connect API key ID}"
  : "${ASC_ISSUER_ID:?set ASC_ISSUER_ID to the App Store Connect issuer ID}"
fi

src_tauri="$(cd "$(dirname "$0")/../src-tauri" && pwd)"
cd "$src_tauri"

export COMMY_VERSION="$version"
if [[ "$version" == *-* ]]; then
  export COMMY_TEST_BUILD=1
  export COMMY_DEFAULT_ACCOUNT_SERVER=https://staging.commy.chat
  export COMMY_DEFAULT_RELAY=https://relay-staging.commy.chat:8443
  export COMMY_RELAY_QAD_PORT=7843
fi
build_number="$(date -u +%y%m%d.%H%M)"
echo "Commy $version, build $build_number"
echo "Account server: ${COMMY_DEFAULT_ACCOUNT_SERVER:-https://commy.chat}"

[ -d gen/apple ] || {
  npx tauri ios init --ci
  npx tauri icon ../mobile/app-icon.png
}

config="$(printf '{"version":"%s","bundle":{"iOS":{"bundleVersion":"%s"}}}' "${version%%-*}" "$build_number")"
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
