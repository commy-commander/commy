#!/usr/bin/env bash
set -euo pipefail

src_tauri="$(cd "$(dirname "$0")/../src-tauri" && pwd)"
project="$src_tauri/gen/apple/project.yml"
[ -f "$project" ] || { echo "run tauri ios init first" >&2; exit 1; }
entitlements="$(sed -n 's/^ *path: *\(.*_iOS\.entitlements\) *$/\1/p' "$project" | head -n 1)"
[ -n "$entitlements" ] || { echo "no entitlements file named in $project" >&2; exit 1; }
node "$(dirname "$0")/ios-project.mjs"
cp "$src_tauri/Entitlements.ios.plist" "$src_tauri/gen/apple/$entitlements"
echo "Entitlements: $entitlements"
