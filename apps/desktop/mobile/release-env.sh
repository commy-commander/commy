#!/usr/bin/env bash

release_version="${1:?usage: source release-env.sh VERSION}"
[[ "$release_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] \
  || { echo "VERSION must look like 0.1.16 or 0.1.16-rc.1" >&2; return 1 2>/dev/null || exit 1; }
release_app_version="${release_version%%-*}"
staging_account_server=https://staging.commy.chat

export_build_value() {
  export "$1=$2" "TAURI_$1=$2"
}

export_build_value COMMY_VERSION "$release_version"
if [[ "$release_version" == *-* ]]; then
  export_build_value COMMY_TEST_BUILD 1
  export_build_value COMMY_DEFAULT_ACCOUNT_SERVER "$staging_account_server"
  export_build_value COMMY_DEFAULT_RELAY https://relay-staging.commy.chat:8443
  export_build_value COMMY_RELAY_QAD_PORT 7843
fi
echo "Commy $release_version"
echo "Account server: ${COMMY_DEFAULT_ACCOUNT_SERVER:-https://commy.chat}"

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
