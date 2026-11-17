#!/usr/bin/env bash
set -euo pipefail

appimage="$(readlink -f "$1")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

(cd "$work" && "$appimage" --appimage-extract >/dev/null)
root="$work/squashfs-root"
plugins="$root/usr/lib/gstreamer-1.0"
hook="$root/apprun-hooks/linuxdeploy-plugin-gstreamer.sh"

fail() {
  echo "AppImage media check failed: $1"
  exit 1
}

for plugin in coreelements video4linux2 pulseaudio videoconvert videoscale videorate audioconvert audioresample app playback mpg123; do
  [ -f "$plugins/libgst$plugin.so" ] || fail "no GStreamer $plugin plugin, so cameras, microphones or sounds would be missing"
done
for unwanted in pipewire soup; do
  [ ! -e "$plugins/libgst$unwanted.so" ] || fail "carries the GStreamer $unwanted plugin, which clashes with the system's own libraries"
done
[ -x "$root/usr/lib/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner" ] || fail "no gst-plugin-scanner"
[ -f "$hook" ] || fail "no GStreamer AppRun hook"
grep -q 'GST_PLUGIN_SYSTEM_PATH_1_0="${APPDIR}/usr/lib/gstreamer-1.0"' "$hook" || fail "the AppRun hook does not point GStreamer at the bundled plugins"
grep -q 'linuxdeploy-plugin-gstreamer.sh' "$root/AppRun" || fail "AppRun does not load the GStreamer hook"

echo "AppImage carries $(ls "$plugins" | wc -l) GStreamer plugins and points GStreamer at them"
