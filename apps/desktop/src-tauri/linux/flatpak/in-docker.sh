#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
image="${FLATPAK_IMAGE:-ghcr.io/flathub-infra/flatpak-github-actions:gnome-51@sha256:78d969b18225ae107ca29497bf09c4a3f791e5f1838a9a912a529d0024fb6210}"
deb="$1"
version="$2"
out="$3"
branch="${4:-stable}"
name="commy-flatpak-$$"

docker create --privileged --name "$name" "$image" sleep infinity >/dev/null
trap 'docker rm -f "$name" >/dev/null 2>&1 || true' EXIT
docker start "$name" >/dev/null
docker exec "$name" mkdir -p /build/recipe
docker cp "$here/." "$name:/build/recipe/"
docker cp "$deb" "$name:/build/commy.deb"
docker exec "$name" bash /build/recipe/build.sh /build/commy.deb "$version" /build/commy.flatpak "$branch"
docker cp "$name:/build/commy.flatpak" "$out"
