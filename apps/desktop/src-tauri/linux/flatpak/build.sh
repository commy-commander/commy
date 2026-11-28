#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
deb="$(realpath "$1")"
version="$2"
out="$(realpath -m "$3")"
branch="${4:-stable}"
app=chat.commy.desktop
shared_modules=https://github.com/flathub/shared-modules.git
shared_modules_commit=34add58f536223b780a3c9f077980cdaa51fc81f
runtime_repo=https://dl.flathub.org/repo/flathub.flatpakrepo

work="$(mktemp -d)"
cp "$here/$app.yml" "$work/"
cp "$deb" "$work/commy.deb"
sed -e "s/@VERSION@/$version/" -e "s/@DATE@/$(date -u +%Y-%m-%d)/" "$here/$app.metainfo.xml" > "$work/$app.metainfo.xml"
appstreamcli validate --no-net --explain "$work/$app.metainfo.xml"

git clone -q --filter=blob:none "$shared_modules" "$work/shared-modules"
git -C "$work/shared-modules" checkout -q "$shared_modules_commit"

cd "$work"
flatpak-builder --disable-rofiles-fuse --force-clean --default-branch="$branch" --repo=repo build "$app.yml"

desktop="build/export/share/applications/$app.desktop"
desktop-file-validate "$desktop"
grep -q '^MimeType=x-scheme-handler/commy' "$desktop" || { echo "the Flatpak does not handle commy:// links"; cat "$desktop"; exit 1; }
grep -q '^Exec=.*%u' "$desktop" || { echo "the Flatpak launcher does not pass links on"; cat "$desktop"; exit 1; }
for file in bin/commy-desktop lib/commy/gstreamer-1.0/libgstcommycamerafix.so lib/libayatana-appindicator3.so.1 share/metainfo/$app.metainfo.xml share/icons/hicolor/256x256/apps/$app.png; do
  [ -e "build/files/$file" ] || { echo "the Flatpak is missing /app/$file"; exit 1; }
done

flatpak build-bundle --runtime-repo="$runtime_repo" repo "$out" "$app" "$branch"
ls -la "$out"
