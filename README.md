# Commy

Free, open source voice and text chat for communities, written in Rust.

Commy servers live on their members' own computers. Chat, voice, files and
screen shares travel directly between the people in a server, so there is no
central service reading your messages and nothing to pay for. The only hosted
pieces are the account server and the website at [commy.chat](https://commy.chat).

## What you can do

- **Servers and channels.** Create a server from scratch or a starter
  template, add text and voice channels, group servers into folders, and
  invite people with a link. Copy any server's channels, roles and rules as a
  template code so others can start from the same layout, and hand a server
  you own to another member.
- **Roles and moderation.** Colored roles with fine-grained permissions,
  kick, ban and timeouts, per-server nicknames, server mute and deafen,
  pinned messages and announcements, and auto moderation that holds messages
  with blocked words, links, too many mentions or flooding. A moderation bot
  you run yourself can remove new ones for everyone
  ([crates/commy-bot](crates/commy-bot/README.md)).
  Channels can be limited to roles. Messages sent in a private channel are
  encrypted so only those roles and admins can read them.
- **Rich chat.** Replies, reactions, edits, emoji (including custom server
  emoji), images and files shown inline, clickable links, polls with a closing
  time, and a GIF picker when the server operator has set up a GIF provider key.
- **Message search.** Search a server by keyword, phrase, author, mention or
  channel. Search runs on your device over the history it keeps, and a result
  jumps straight to the message, even weeks back.
- **Voice.** Opus voice with one-click noise cancellation and echo
  cancellation toggles, per-person volume, shared mute and deafen, and a
  soundboard with per-server clips.
- **Screen and game streaming.** Share a whole monitor or a single window with
  a Sharp, Balanced or Smooth quality preset. Windows uses Windows Graphics
  Capture, so games stream cleanly, and can send what your computer plays
  along with the picture; viewers set their own stream volume.
- **Direct messages.** Private conversations between two people, delivered
  straight to their devices, with offline messages that arrive when the other
  person comes back.
- **Presence and notifications.** Status, Do Not Disturb with a "back at"
  time, appear offline, notification toasts, and a notification level or a
  timed mute for each server and each channel.
- **Game overlay.** On Windows, an in-game overlay shows who is talking and
  recent chat, in the screen corner you choose.
- **Safety tools.** Block and report people, an audit log of server changes,
  and invite links that expire or can be revoked.

Windows is the primary platform today. macOS and Linux builds are published
with each release; a few features, such as the game overlay and notification
toasts, are Windows only for now.

## Download

Releases are on the [Releases page](https://github.com/commy-commander/commy/releases)
and at [commy.chat](https://commy.chat). Release candidates (`-rc`) are
pre-releases for testing and connect only to the staging servers.

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
crates/commy-core       Domain model: servers, channels, messages,
                        permissions, replicated state (no I/O)
crates/commy-net        Peer to peer transport: invite tickets, gossip
                        protocol, node wrapper, blob transfer
crates/commy-audio      Voice processing: noise cancellation, mute gate,
                        voice activity detection, Opus codec
crates/commy-media      Device I/O: microphone, playback mixing with jitter
                        buffers, screen and window capture
crates/commy-bot        Headless member that enforces auto moderation
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
instance with its own data directory and join your own server with an invite:

```powershell
$env:COMMY_DATA_DIR = "$env:APPDATA\chat.commy.desktop-2"
cargo run -p commy-desktop
```

`auth/docker-compose.dev.yml` runs a local account server with a mail catcher
and Redis.

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
