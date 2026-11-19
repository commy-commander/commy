# Building Commy

## Prerequisites

You need Rust (stable), Node.js and the Tauri 2 CLI.

Windows (PowerShell):

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --passive"
winget install OpenJS.NodeJS.LTS
winget install Git.Git
rustup default stable
cargo install tauri-cli --version "^2"
```

macOS:

```sh
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
brew install node
cargo install tauri-cli --version "^2"
```


## Build and run

```sh
git clone https://git.commy.chat/commy/commy.git
cd commy

# Run the Rust tests (core, net, audio, media, bot)
cargo test --workspace

# Run the desktop app
cd apps/desktop/src-tauri
cargo tauri dev
```

The frontend is embedded at build time, so UI edits need a rebuild. To preview
the UI alone in a browser with a mock backend, open `apps/desktop/ui/index.html`.

The web client runs the same UI in the browser. Building it needs the
`wasm32-unknown-unknown` target and `wasm-bindgen-cli` at the version in
`Cargo.lock`; see [apps/web/README.md](../apps/web/README.md).

## Testing on one machine

To test voice, streaming and direct messages on one machine, run a second
instance with its own data directory and join your own server with an invite.
Commy runs one copy per data directory, so starting it again with the same
one only brings the open window forward:

```powershell
$env:COMMY_DATA_DIR = "$env:APPDATA\chat.commy.desktop-2"
cargo run -p commy-desktop
```

`auth/docker-compose.dev.yml` runs a local account server with a mail catcher
and Redis.

## Phone apps (early)

Android and iOS build from the same crate and UI. The UI switches to a phone
layout below 700px wide; add `?mobile` to the preview URL to also hide the
desktop-only settings. The native projects are generated, not committed:

```sh
cd apps/desktop/src-tauri

npm ci

# Android (needs Android Studio's SDK and NDK, NDK_HOME set)
npx tauri android init
npx tauri icon ../mobile/app-icon.png
node ../mobile/android-project.mjs
npx tauri android dev

# iOS (a Mac with Xcode)
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
npx tauri ios init
npx tauri icon ../mobile/app-icon.png
npx tauri ios dev
```

`npx tauri ios dev` lists the simulators and any plugged-in iPhone or iPad. A
real device needs your Apple team ID in `APPLE_DEVELOPMENT_TEAM` (or picked in
Xcode under Signing & Capabilities after `npx tauri ios dev --open`).
`apps/desktop/mobile/ios-build.sh 0.1.16` builds an App Store signed IPA with
the Apple account signed in to Xcode, and `--testflight` also uploads it, using
the App Store Connect API key named by `ASC_KEY_ID` and `ASC_ISSUER_ID` with its
`.p8` in `~/.appstoreconnect/private_keys`. An `-rc` version builds against the
staging servers.

The "Mobile builds" workflow builds a debug APK on every change to native or
phone files; iOS builds only when the workflow is run by hand. Its `ios` input
picks `simulator` (an Apple silicon simulator app plus an unsigned device build),
`signed` or `testflight`, which run `ios-build.sh` with the `version` input.
The iOS job runs on the Mac runner labelled `commy-mac`. Signing needs the
`APPLE_DEVELOPMENT_TEAM` secret and, with nothing else set, uses the Apple
account in that Mac's Xcode settings. Without one, set either an App Store
Connect API key with Admin access (`APPLE_API_KEY_ID`, `APPLE_API_ISSUER`, and
the `.p8` contents in `APPLE_API_KEY`) or a distribution certificate and profile
(`IOS_CERTIFICATE` and `IOS_MOBILE_PROVISION` as base64,
`IOS_CERTIFICATE_PASSWORD`). TestFlight needs `APPLE_API_KEY_ID` and
`APPLE_API_ISSUER`, with the key's `.p8` either in `APPLE_API_KEY` or in
`~/.appstoreconnect/private_keys` on the Mac. To let the sync service wake the Android app with an empty push, set
`GOOGLE_SERVICES_JSON` to your Firebase project's `google-services.json` (its
contents, its base64 or a path) before running `android-project.mjs`; the workflow reads it
from the secret of the same name. Without it debug APKs build without push
wake-ups (with a warning), and Play bundles stop, because a release app must
wake phones (`COMMY_PUSH_REQUIRED=1`). CI signs each debug APK with a fresh key, so installing a newer one
means uninstalling the old one first; to keep one key, store a debug keystore
(password and key password `android`, alias `androiddebugkey`) as base64 in the
`ANDROID_DEBUG_KEYSTORE` secret. Running the workflow with `android: play` and a
`version` runs `apps/desktop/mobile/android-build.sh`, which builds a release
bundle for Google Play and a release APK, signed with the upload key from the
`ANDROID_UPLOAD_KEYSTORE` (base64 PKCS12, alias `commy-upload`) and
`ANDROID_UPLOAD_PASSWORD` secrets; without them it signs with a throwaway key
that Google Play rejects. When the `PLAY_SERVICE_ACCOUNT_JSON` secret holds a
Google service account key with permission to release to testing tracks,
`apps/desktop/mobile/play-upload.mjs` then sends the bundle to the Play testing
track picked in `play_testing` (internal by default, or closed or open). Test
builds with an `-rc` version are not sent, and production releases stay a manual
step in Play Console. Screen sharing, the overlay, hotkeys, the
tray and Stream Deck are desktop only.
