#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here}"
mkdir -p "$out"

gcc -shared -fPIC -O2 -Wall -Wextra -Werror -Wno-unused-parameter -Wno-missing-field-initializers \
  -DPACKAGE='"commy"' -o "$out/libgstcommycamerafix.so" "$here/commycamerafix.c" \
  $(pkg-config --cflags --libs gstreamer-1.0)

echo "built $out/libgstcommycamerafix.so"
