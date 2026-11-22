#!/usr/bin/env bash

release_version="${1:?usage: source release-env.sh VERSION}"
[[ "$release_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] \
  || { echo "VERSION must look like 0.1.16 or 0.1.16-rc.1" >&2; return 1 2>/dev/null || exit 1; }
release_app_version="${release_version%%-*}"
staging_account_server=https://staging.commy.chat
release_mobile_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export_build_value() {
  export "$1=$2" "TAURI_$1=$2"
}

release_ios_bundle_id=chat.commy.mobile
release_app_name=Commy
release_app_icon="$release_mobile_dir/app-icon.png"
release_ios_info_plist=""

export_build_value COMMY_VERSION "$release_version"
if [[ "$release_version" == *-* ]]; then
  export_build_value COMMY_TEST_BUILD 1
  export_build_value COMMY_DEFAULT_ACCOUNT_SERVER "$staging_account_server"
  export_build_value COMMY_DEFAULT_RELAY https://relay-staging.commy.chat:8443
  export_build_value COMMY_RELAY_QAD_PORT 7843
  release_ios_bundle_id=chat.commy.mobile.beta
  release_app_name="Commy Beta"
  release_app_icon="$release_mobile_dir/app-icon-beta.png"
  release_ios_info_plist="$release_mobile_dir/ios/beta/Info.plist"
fi
export COMMY_IOS_BUNDLE_ID="$release_ios_bundle_id"
echo "Commy $release_version"
echo "Account server: ${COMMY_DEFAULT_ACCOUNT_SERVER:-https://commy.chat}"
echo "iPhone app: $release_app_name ($release_ios_bundle_id)"

ios_build_config() {
  local bundle_version="${1:-}"
  local ios=()
  [[ -z "$bundle_version" ]] || ios+=("$(printf '"bundleVersion":"%s"' "$bundle_version")")
  [[ -z "$release_ios_info_plist" ]] || ios+=("$(printf '"infoPlist":"%s"' "$release_ios_info_plist")")
  local joined
  joined="$(IFS=,; printf '%s' "${ios[*]}")"
  printf '{"identifier":"%s","version":"%s","bundle":{"iOS":{%s}}}' "$release_ios_bundle_id" "$release_app_version" "$joined"
}

expect_ios_bundle_id() {
  local label="$1" built
  built="$(plutil -extract CFBundleIdentifier raw -o - - 2>/dev/null || true)"
  if [[ "$built" != "$release_ios_bundle_id" ]]; then
    echo "::error::$label was built as ${built:-an unknown app} but $release_version must be $release_ios_bundle_id" >&2
    return 1
  fi
  echo "$label is $release_app_name ($built)"
}

expect_built_servers() {
  local label="$1"
  local matches found=no
  matches="$(grep -ac "$staging_account_server" || true)"
  [[ "${matches:-0}" -gt 0 ]] && found=yes
  if [[ "$release_version" == *-* && "$found" == no ]]; then
    echo "::error::$label was built for $release_version but does not point at $staging_account_server" >&2
    return 1
  fi
  if [[ "$release_version" != *-* && "$found" == yes ]]; then
    echo "::error::$label was built for $release_version but points at $staging_account_server" >&2
    return 1
  fi
  echo "$label points at ${COMMY_DEFAULT_ACCOUNT_SERVER:-https://commy.chat}"
}
