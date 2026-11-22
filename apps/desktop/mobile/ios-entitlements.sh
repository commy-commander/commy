#!/usr/bin/env bash
set -euo pipefail

src_tauri="$(cd "$(dirname "$0")/../src-tauri" && pwd)"
project="$src_tauri/gen/apple/project.yml"
[ -f "$project" ] || { echo "run tauri ios init first" >&2; exit 1; }
node "$(dirname "$0")/ios-project.mjs"
