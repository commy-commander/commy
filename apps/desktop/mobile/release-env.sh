#!/usr/bin/env bash

release_version="${1:?usage: source release-env.sh VERSION}"
[[ "$release_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] \
  || { echo "VERSION must look like 0.1.16 or 0.1.16-rc.1" >&2; return 1 2>/dev/null || exit 1; }
release_app_version="${release_version%%-*}"

export COMMY_VERSION="$release_version"
if [[ "$release_version" == *-* ]]; then
  export COMMY_TEST_BUILD=1
  export COMMY_DEFAULT_ACCOUNT_SERVER=https://staging.commy.chat
  export COMMY_DEFAULT_RELAY=https://relay-staging.commy.chat:8443
  export COMMY_RELAY_QAD_PORT=7843
fi
echo "Commy $release_version"
echo "Account server: ${COMMY_DEFAULT_ACCOUNT_SERVER:-https://commy.chat}"
