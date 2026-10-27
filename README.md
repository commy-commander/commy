# Commy

Free, open source voice and text chat for communities, written in Rust.

Commy servers live on their members' own computers. Chat, voice, files and
screen shares travel directly between the people in a server, so there is no
central service reading your messages and nothing to pay for. The only hosted
pieces are the account server and the website at [commy.chat](https://commy.chat).

## What you can do

- **Servers and channels.** Create a server from scratch or a starter
  template, add text and voice channels, forum channels for posts with tags,
  group servers into folders, and invite people with a link, and list a
  public server in Discover so others can find it. A Ctrl+K quick switcher jumps to any channel, conversation or
  server by name. Mark a channel age-restricted and it asks people to confirm
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
  has set up a GIF provider key.
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
  voice channel, and a soundboard with per-server clips. Invite a friend or
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

## Download

Releases are on the [Releases page](https://github.com/commy-commander/commy/releases)
and at [commy.chat](https://commy.chat). Release candidates (`-rc`) are
pre-releases for testing and connect only to the staging servers. Each release
also carries `commy-bot` for Windows, macOS and Linux, for running bots; see the
[bot guide](crates/commy-bot/README.md).

### Verifying a download

Every release ships `SHA256SUMS.txt` with the hash of each file, signed by the
release workflow with [cosign](https://github.com/sigstore/cosign) (keyless,
through GitHub's OIDC identity), and every file carries a GitHub build
provenance attestation. With the file, `SHA256SUMS.txt`, `SHA256SUMS.txt.sig`
and `SHA256SUMS.txt.pem` in one folder:

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

## How it works

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
                        voice activity detection, Opus codec
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

- Every running app is an [iroh](https://iroh.computer) endpoint identified by
  an Ed25519 key. Connections are direct and encrypted; relays are only a
  fallback transport for people who cannot reach each other directly, never a
  place where content is stored.
- A Commy server is a gossip topic. Every change is an event broadcast to the
  topic and applied idempotently to each member's copy of the server state.
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
- Voice and screen shares use direct media connections between members. The
  account server never sees messages, files, voice, video or which servers an
  account belongs to. What it does hold is listed in
  [deploy/HOSTING.md](deploy/HOSTING.md#what-the-operator-can-see).

## Build from source

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

Then:

```sh
git clone https://github.com/commy-commander/commy.git
cd commy

# Run the Rust tests (core, net, audio, media, bot)
cargo test --workspace

# Run the desktop app
cd apps/desktop/src-tauri
cargo tauri dev
```

The frontend is embedded at build time, so UI edits need a rebuild. To preview
the UI alone in a browser with a mock backend, open `apps/desktop/ui/index.html`.

To test voice, streaming and direct messages on one machine, run a second
instance with its own data directory and join your own server with an invite.
Commy runs one copy per data directory, so starting it again with the same
one only brings the open window forward:

```powershell
$env:COMMY_DATA_DIR = "$env:APPDATA\chat.commy.desktop-2"
cargo run -p commy-desktop
```

The web client is an early preview with text channels only. Building it needs
the `wasm32-unknown-unknown` target and `wasm-bindgen-cli` at the version in
`Cargo.lock`; see [apps/web/README.md](apps/web/README.md).

`auth/docker-compose.dev.yml` runs a local account server with a mail catcher
and Redis.

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

# iOS (macOS with Xcode)
npx tauri ios init
npx tauri icon ../mobile/app-icon.png
npx tauri ios dev
```

The "Mobile builds" workflow builds a debug APK on every change to native or
phone files; the iOS simulator app builds only when the workflow is run by
hand. To let the sync service wake the Android app with an empty push, set
`GOOGLE_SERVICES_JSON` to your Firebase project's `google-services.json` (its
contents, its base64 or a path) before running `android-project.mjs`; the workflow reads it
from the secret of the same name. Without it the app builds without push
wake-ups. CI signs each debug APK with a fresh key, so installing a newer one
means uninstalling the old one first; to keep one key, store a debug keystore
(password and key password `android`, alias `androiddebugkey`) as base64 in the
`ANDROID_DEBUG_KEYSTORE` secret. Screen sharing, the overlay, hotkeys, the
tray and Stream Deck are desktop only.

## Self-hosting

You do not need to host anything to use Commy. If you want your own account
server, [deploy/HOSTING.md](deploy/HOSTING.md) covers the Docker Compose stack,
sign-in providers, optional GIF search, and pointing the desktop app at your
server. [deploy/DEPLOYING.md](deploy/DEPLOYING.md) describes the deploy
pipeline.

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) for the
branch flow and checks, and the [Code of Conduct](CODE_OF_CONDUCT.md) for how we
treat each other. Found a bug or have an idea? Open an
[issue](https://github.com/commy-commander/commy/issues/new/choose). To report
a security problem, follow [SECURITY.md](SECURITY.md) instead of opening a
public issue.

## License

Commy is licensed under the [GNU Affero General Public License v3.0](LICENSE).
