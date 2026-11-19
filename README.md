<a id="readme-top"></a>

<div align="center">

[![License: AGPL-3.0][license-shield]][license-url]
[![Latest release][release-shield]][release-url]
[![Support Commy on Ko-fi][kofi-shield]][kofi-url]

<br />

<a href="https://commy.chat">
  <img src="auth/pages/src/logo.svg" alt="Commy logo" width="96" height="96">
</a>

<h1 align="center">Commy</h1>

<p align="center">
  Free, open source voice and text chat for communities, written in Rust.
  <br />
  <a href="https://commy.chat"><strong>Visit commy.chat »</strong></a>
  <br />
  <br />
  <a href="https://commy.chat/download">Download</a>
  &middot;
  <a href="https://github.com/commy-commander/commy/issues/new?template=bug_report.yml">Report a bug</a>
  &middot;
  <a href="https://github.com/commy-commander/commy/issues/new?template=feature_request.yml">Request a feature</a>
  &middot;
  <a href="https://ko-fi.com/commychat">Support on Ko-fi</a>
</p>

</div>

<details>
  <summary>Table of contents</summary>
  <ol>
    <li>
      <a href="#about-the-project">About the project</a>
      <ul>
        <li><a href="#what-you-can-do">What you can do</a></li>
        <li><a href="#built-with">Built with</a></li>
        <li><a href="#how-it-works">How it works</a></li>
      </ul>
    </li>
    <li>
      <a href="#getting-started">Getting started</a>
      <ul>
        <li><a href="#download">Download</a></li>
        <li><a href="#verifying-a-download">Verifying a download</a></li>
        <li><a href="#prerequisites">Prerequisites</a></li>
        <li><a href="#build-from-source">Build from source</a></li>
        <li><a href="#phone-apps-early">Phone apps (early)</a></li>
      </ul>
    </li>
    <li>
      <a href="#usage">Usage</a>
      <ul>
        <li><a href="#self-hosting">Self-hosting</a></li>
      </ul>
    </li>
    <li><a href="#roadmap">Roadmap</a></li>
    <li><a href="#contributing">Contributing</a></li>
    <li><a href="#support-commy">Support Commy</a></li>
    <li><a href="#license">License</a></li>
    <li><a href="#contact">Contact</a></li>
    <li><a href="#acknowledgments">Acknowledgments</a></li>
  </ol>
</details>

## About the project

Commy is a place for communities to talk: text channels, voice channels,
screen and game streaming, and direct messages, in a desktop app, a browser
client and early phone apps.

Voice, video, screen shares and files travel directly between the people in a
server. Messages and server settings sync through a sync service that keeps
each community's data sealed end to end, so members who were offline catch up
and nobody running the service can read what is said. The account server never
sees messages, files, voice, video or which servers an account belongs to.
Commy is free to use, and anyone can run their own sync host.

### What you can do

- **Servers and channels.** Create a server from scratch or a starter
  template, add text and voice channels, forum channels for posts with tags,
  group servers into folders, and invite people with a link, and list a
  public server in Discover so others can find it. A Ctrl+K quick switcher
  jumps to any channel, conversation or server by name. Mark a channel age-restricted and it asks people to confirm
  they are 18 or older before showing anything. Copy any server's channels,
  roles and rules as a template code so others can start from the same
  layout, and hand a server you own to another member. Onboarding questions
  ask new members what they are here for and hand out the roles, and so the
  channels, that match; answers can be changed later.
- **Roles and moderation.** Colored roles with fine-grained permissions,
  down to who may embed links, read message history, start threads, post
  polls or schedule events, kick, ban and timeouts, per-server nicknames, server mute and deafen,
  moving people between voice channels from their menu or by dragging them,
  pinned messages and announcements,
  and auto moderation that holds messages with blocked words, links, too many
  mentions or flooding. A moderation bot you run yourself can remove new ones
  for everyone ([crates/commy-bot](crates/commy-bot/README.md)), and bots can
  register slash commands that members pick from the composer with option
  hints; using one posts a compact "used /name" line that the bot answers.
  Channels can be limited to roles. Messages sent in a private channel are
  encrypted so only those roles and admins can read them.
- **Rich chat.** Replies, reactions, edits, named threads that can be
  archived or locked, emoji (including custom server emoji), images and
  files shown inline, voice messages recorded from the composer and played
  back in place, `/tts` messages that readers can have read aloud, clickable
  links, polls with a closing time, and a GIF picker when the server operator
  has set up a GIF provider key. A server owner can connect file storage
  on their own Cloudflare account so shared files load even while everyone
  who has them is offline ([apps/file-store](apps/file-store/README.md)).
- **Message search.** Search a server by keyword, phrase, author, mention or
  channel, or your direct messages and groups, scoped to the conversation you
  are viewing or all of them. Search runs on your device over the history it
  keeps, pages through results fifty at a time newest first, and a result
  jumps straight to the message, even weeks back. Save any message to a
  per-device Saved list, or ask for a reminder about it in 20 minutes, an
  hour, three hours, tomorrow morning or at a time you pick; the reminder
  shows a notification when it is due.
- **Voice.** Opus voice with one-click noise cancellation and echo
  cancellation toggles, per-person volume, shared mute and deafen, priority
  speakers who turn everyone else down while they talk, a status line per
  voice channel, a soundboard with per-server clips, and voice filters
  (higher, lower, robot, radio, cave and whisper) that a server can switch
  off for some roles or everyone. Invite a friend or
  a channel to a call with a card that shows who is in it right now and a
  Join button. Stage channels seat an audience that only listens and can
  raise a hand; hosts invite people up to speak. Activities in a
  voice channel: watch a direct video link or an attached video together,
  kept within a second of each other, or play tic-tac-toe while the rest of
  the channel watches.
- **Screen and game streaming.** Share a whole monitor or a single window with
  a Sharp, Balanced or Smooth quality preset. Windows uses Windows Graphics
  Capture, so games stream cleanly, and can send what your computer plays
  along with the picture; viewers set their own stream volume.
- **Direct messages.** Private conversations between two people, delivered
  straight to their devices, with offline messages that arrive when the other
  person comes back. Pin the conversations you use most, and the channels too,
  so they stay at the top of the list on that device.
- **Presence and notifications.** Status, Do Not Disturb with a "back at"
  time, appear offline, notification toasts, and a notification level or a
  timed mute for each server and each channel. An optional toast when a
  friend comes online, for everyone or just the few you pick, quiet while
  you are Do Not Disturb. A red "NEW" line marks where
  you left off in a channel, any message can be marked unread, and read
  positions follow you between your devices in the same encrypted sync as
  your server list.
- **Game overlay.** On Windows, an in-game overlay shows who is talking and
  recent chat, in the screen corner you choose.
- **Settings you can find.** Settings are grouped into You, Voice & Video,
  Alerts & Sounds, App and Advanced, and a search box at the top of the list
  jumps straight to any setting.
- **Accessibility.** Settings > Accessibility has reduced motion (on by
  default when your system asks for it), a colour saturation slider for role
  and status colours, and high contrast. Settings > Appearance has a light
  theme or follow-system theme and a language picker. Settings, navigation and sign-in are translated
  into Spanish so far; the dictionaries live in
  `apps/desktop/ui/scripts/locales/`, one file per language, and the rest of
  the app can be translated the same way. Everything here is per device.
- **Safety tools.** Block and report people, an audit log of server changes,
  and invite links that expire or can be revoked. Choose who can see your
  profile details (everyone, friends and the servers you share, or friends
  only); your name and avatar always show. Friend requests can carry a short
  note so the other person knows who is asking.

Windows is the primary platform today. macOS and Linux builds are published
with each release; a few features, such as the game overlay and notification
toasts, are Windows only for now.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

### Built with

[![Rust][rust-shield]][rust-url]
[![Tauri][tauri-shield]][tauri-url]
[![WebAssembly][wasm-shield]][wasm-url]
[![Node.js][node-shield]][node-url]
[![PostgreSQL][postgres-shield]][postgres-url]
[![Redis][redis-shield]][redis-url]
[![Docker][docker-shield]][docker-url]

The desktop and phone apps are a Rust core inside a Tauri 2 shell, with a
frontend in plain HTML, CSS and JavaScript. The browser client runs the same
frontend on a WebAssembly build of the same Rust code. The account server uses
Better Auth on Node.js with Postgres and Redis, and the sync service is Rust on
Postgres. Voice uses the Opus codec.

### How it works

```
apps/desktop            Tauri 2 desktop app
  src-tauri             Rust shell: commands, app state, network event loop
  ui                    Frontend (plain HTML, CSS and JavaScript)
apps/web                Browser build: the same UI on a WebAssembly backend
crates/commy-core       Domain model: servers, channels, messages,
                        permissions, replicated state (no I/O)
crates/commy-net        Peer to peer transport: invite tickets, gossip
                        protocol, node wrapper, blob transfer
crates/commy-view       The view models the UI renders, shared by both apps
crates/commy-audio      Voice processing: noise cancellation, mute gate,
                        voice activity detection, voice filters, Opus codec
crates/commy-media      Device I/O: microphone, playback mixing with jitter
                        buffers, screen and window capture
crates/commy-bot        Headless member that enforces auto moderation
crates/commy-sync       Sealed sync streams: key derivation, padding, wire
                        format and a native client
crates/commy-sync-server
                        Sync service that orders and fans out sealed
                        entries it cannot read
auth                    Account server: Better Auth, Postgres, Redis
deploy                  Self-hosting stack and deploy pipeline
```

- Every running app is identified by an Ed25519 key. Connections between
  members are direct and encrypted; relays are only a fallback for people who
  cannot reach each other directly, never a place where content is stored.
- A Commy server is a gossip topic. Every change is an event broadcast to the
  topic and applied idempotently to each member's copy of the server state.
- Messages and server state also go through the sync service as sealed
  entries, so they reach members who were offline. The service orders and
  fans them out without being able to read them.
- Invites are self-contained tickets. Short invite links store the ticket
  sealed with a key that only the link carries, so the account server cannot
  read it.
- Files, avatars, emoji and soundboard clips move as content-addressed blobs;
  chat messages carry only the hash.
- What the app keeps on disk (messages, direct messages, keys, the sign-in
  token, cached pictures) is encrypted with AES-256-GCM under a key that
  Windows (DPAPI) or the macOS Keychain holds for your user account. If that
  key is ever lost, the unreadable files are moved to an `unreadable` folder
  instead of being deleted, and a signed-in account can get its identity and
  server list back from another device signed in to it.
- Voice and screen shares use direct media connections between members. What
  the account server does hold is listed in
  [deploy/HOSTING.md](deploy/HOSTING.md#what-the-operator-can-see).

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Getting started

### Download

Downloads are at [commy.chat/download](https://commy.chat/download), served from
releases.commy.chat: every version keeps its files under its tag, such as
`https://releases.commy.chat/v0.2.0/`, and `stable/release.json` and `rc/release.json`
name the newest final and release candidate. Release candidates (`-rc`) are
pre-releases for testing and connect only to the staging servers. Each release
also carries `commy-bot` for Windows, macOS and Linux, for running bots; see the
[bot guide](crates/commy-bot/README.md).

### Verifying a download

Every release ships `SHA256SUMS.txt` with the hash of each file, signed by the
release workflow with [cosign](https://github.com/sigstore/cosign).

Releases built on git.commy.chat sign it with the Commy release key,
[`deploy/agent/cosign.pub`](deploy/agent/cosign.pub), the same key that signs
the server images. With the file, `SHA256SUMS.txt`, `SHA256SUMS.txt.bundle` and
`cosign.pub` in one folder:

```bash
cosign verify-blob --key cosign.pub --bundle SHA256SUMS.txt.bundle SHA256SUMS.txt
sha256sum --check --ignore-missing SHA256SUMS.txt
```

The first command proves the checksum list was signed with the release key, the
second that your file matches it.

Releases built on GitHub were signed keyless, through GitHub's OIDC identity,
and every file carries a GitHub build provenance attestation. With the file,
`SHA256SUMS.txt`, `SHA256SUMS.txt.sig` and `SHA256SUMS.txt.pem` in one folder:

```bash
cosign verify-blob --certificate SHA256SUMS.txt.pem --signature SHA256SUMS.txt.sig \
  --certificate-identity-regexp '^https://github.com/commy-commander/commy/\.github/workflows/release\.yml@refs/tags/v' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS.txt
sha256sum --check --ignore-missing SHA256SUMS.txt
gh attestation verify Commy_<version>_x64-setup.exe --repo commy-commander/commy
```

The first command proves the checksum list was produced by this repository's
release workflow on a release tag, the second that your file matches it, and
the third (with the [GitHub CLI](https://cli.github.com)) that the file itself
was built by that workflow from this repository.

Builds are not code signed yet, so Windows SmartScreen and macOS Gatekeeper
will ask you to confirm before the first launch.


### Prerequisites

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


### Build from source

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
`Cargo.lock`; see [apps/web/README.md](apps/web/README.md).

### Phone apps (early)

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


<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Usage

Download Commy, create an account, then create a server from scratch or a
starter template and share an invite link with your friends. The guides at
[commy.chat](https://commy.chat) walk through the rest.

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

### Self-hosting

You do not need to host anything to use Commy. To keep your communities'
messages and history on your own server, run a sync host; to run accounts, the
website and sync on your own domain, run the whole stack.
[deploy/HOSTING.md](deploy/HOSTING.md) covers both, along with sign-in
providers, optional GIF search, and pointing the desktop app at your server.
[deploy/DEPLOYING.md](deploy/DEPLOYING.md) describes the deploy pipeline.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Roadmap

- [x] Desktop app for Windows, macOS and Linux
- [x] Browser client running the same UI
- [x] Run your own sync host
- [ ] Android and iPhone apps in Google Play and the App Store (in testing now)
- [ ] Code signed Windows builds
- [ ] The whole app translated, starting with Spanish

See the [open issues](https://github.com/commy-commander/commy/issues) for
requested features and known problems.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) for the
branch flow and checks, and the [Code of Conduct](CODE_OF_CONDUCT.md) for how we
treat each other. Found a bug or have an idea? Open an
[issue](https://github.com/commy-commander/commy/issues/new/choose). To report
a security problem, follow [SECURITY.md](SECURITY.md) instead of opening a
public issue.

1. Cut a branch from the latest `main`, named after the change's type
   (`git switch -c feat/my-change`)
2. Commit with [Conventional Commits](https://www.conventionalcommits.org)
   (`git commit -m "feat(ui): add my change"`)
3. Run `cargo clippy --workspace` and `cargo test --workspace`
4. Push the branch (`git push -u origin feat/my-change`)
5. Open a pull request into `main`

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Support Commy

Commy is free to use. If it is useful to you or your community, you can help
keep the servers running and the work going on
[Ko-fi](https://ko-fi.com/commychat).

[![Support Commy on Ko-fi][kofi-shield]][kofi-url]

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## License

Commy is licensed under the [GNU Affero General Public License v3.0](LICENSE).

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Contact

Commy Commander

- Website: [commy.chat](https://commy.chat)
- Help with the app: [support@commy.chat](mailto:support@commy.chat)
- Security problems: see [SECURITY.md](SECURITY.md)
- Source: [git.commy.chat/commy/commy](https://git.commy.chat/commy/commy)

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Acknowledgments

- [Tauri](https://tauri.app)
- [Opus](https://opus-codec.org)
- [nnnoiseless](https://github.com/jneem/nnnoiseless)
- [Better Auth](https://www.better-auth.com)
- [Sigstore cosign](https://github.com/sigstore/cosign)
- [Shields.io](https://shields.io)
- [Best-README-Template](https://github.com/othneildrew/Best-README-Template)

<p align="right">(<a href="#readme-top">back to top</a>)</p>

[license-shield]: https://img.shields.io/badge/license-AGPL--3.0-blue.svg?style=for-the-badge
[license-url]: LICENSE
[release-shield]: https://img.shields.io/badge/download-commy.chat-3355ff.svg?style=for-the-badge
[release-url]: https://commy.chat/download
[kofi-shield]: https://img.shields.io/badge/Ko--fi-support%20Commy-FF5E5B.svg?style=for-the-badge&logo=ko-fi&logoColor=white
[kofi-url]: https://ko-fi.com/commychat
[rust-shield]: https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white
[rust-url]: https://www.rust-lang.org
[tauri-shield]: https://img.shields.io/badge/Tauri_2-24C8D8?style=for-the-badge&logo=tauri&logoColor=white
[tauri-url]: https://tauri.app
[wasm-shield]: https://img.shields.io/badge/WebAssembly-654FF0?style=for-the-badge&logo=webassembly&logoColor=white
[wasm-url]: https://webassembly.org
[node-shield]: https://img.shields.io/badge/Node.js-339933?style=for-the-badge&logo=nodedotjs&logoColor=white
[node-url]: https://nodejs.org
[postgres-shield]: https://img.shields.io/badge/PostgreSQL-4169E1?style=for-the-badge&logo=postgresql&logoColor=white
[postgres-url]: https://www.postgresql.org
[redis-shield]: https://img.shields.io/badge/Redis-DC382D?style=for-the-badge&logo=redis&logoColor=white
[redis-url]: https://redis.io
[docker-shield]: https://img.shields.io/badge/Docker-2496ED?style=for-the-badge&logo=docker&logoColor=white
[docker-url]: https://www.docker.com
