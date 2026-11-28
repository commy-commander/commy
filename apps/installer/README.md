# Commy Setup

The small window people download to install Commy on Windows. It shows the Commy star in light while it:

1. Reads `https://releases.commy.chat/stable/latest.json`, the same file the app's self-update reads.
2. Downloads the Windows installer listed there.
3. Checks the download against its signature with the Commy release key, the same key the app's updater trusts. A file that does not match is never run.
4. Runs the normal Commy installer silently (`/S`, plus `/NS` when the desktop shortcut is switched off). Commy installs for the current user only, so Windows never asks for an administrator.
5. Opens Commy. If the person switched off "Start Commy when I sign in", Commy is started with `--no-start-with-computer`.

On a Windows computer without the WebView2 runtime, the window cannot open. Commy Setup then downloads and checks the same installer and opens it with its own window, which adds WebView2.

## Building

```sh
cargo build --release -p commy-installer
```

The release workflow builds it on Windows for every release and publishes it as `Commy-Setup.exe`, with a copy at `https://releases.commy.chat/stable/Commy-Setup.exe`. Release candidates build it with `COMMY_INSTALL_CHANNEL=rc`, so it installs the newest release candidate instead. `COMMY_RELEASES_URL` points it at another release server.

The window is plain HTML in `ui/`. `star.js` draws the star with WebGL and needs no library. The font is Geist, under the SIL Open Font License in `ui/fonts/OFL.txt`.
