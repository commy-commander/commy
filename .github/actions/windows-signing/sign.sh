#!/usr/bin/env bash
set -euo pipefail

export MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*'

for file in "$@"; do
  target="$(cygpath -w "$file")"
  if ! "$WINDOWS_SIGNTOOL" verify /pa /q "$target" >/dev/null 2>&1; then
    "$WINDOWS_SIGNTOOL" sign /fd SHA256 /tr http://timestamp.acs.microsoft.com /td SHA256 \
      /dlib "$WINDOWS_SIGN_DLIB" /dmdf "$WINDOWS_SIGN_METADATA" "$target"
  fi
  "$WINDOWS_SIGNTOOL" verify /pa /v "$target"
done
