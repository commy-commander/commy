#!/usr/bin/env bash
set -euo pipefail

package="$1"
contents="$(rpm -qlp "$package")"
for path in /usr/bin/commy-desktop /usr/lib/commy/gstreamer-1.0/libgstcommycamerafix.so /usr/share/applications/Commy.desktop; do
  grep -qxF "$path" <<< "$contents" || { echo "the .rpm is missing $path"; echo "$contents"; exit 1; }
done

requires="$(rpm -qpR "$package")"
for library in 'libwebkit2gtk-4.1.so.0()(64bit)' 'libgtk-3.so.0()(64bit)' 'libayatana-appindicator3.so.1()(64bit)'; do
  grep -qxF "$library" <<< "$requires" || { echo "the .rpm does not ask for $library"; echo "$requires"; exit 1; }
done

echo "the .rpm carries the app and the camera fix, and asks for WebKitGTK, GTK and the tray library"
