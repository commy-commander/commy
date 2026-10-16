# Commy

Commy is free, open source, peer-to-peer voice and text chat for communities, written in Rust.
Voice, text, file sharing, soundboards, emojis, and screen sharing with no
paywall. Servers live on their members' machines over [iroh](https://iroh.computer);
the only hosted pieces are the website and optional account auth.

## Architecture

```
apps/desktop            Tauri 2 desktop app (Windows + macOS)
  src-tauri             Rust shell: commands, app state, net event loop
  ui                    Frontend (plain HTML/CSS/JS)
crates/commy-core       Domain model: servers, channels, messages,
                        permissions, replicated state (no I/O)
crates/commy-net        P2P transport: invite tickets, gossip protocol,
                        iroh node wrapper, blob transfer
crates/commy-audio      Voice DSP: RNNoise noise cancellation toggle,
                        mute gate, VAD, 48 kHz / 10 ms frames
crates/commy-media      Device I/O: mic capture, playback mixing with
                        jitter buffers, screen capture (MJPEG)
auth                    Account server: Better Auth, Postgres
deploy                  Self-hosting stack and deploy pipeline
                        (the commy.chat website lives in its own
                        repository, commy-web)
```

How P2P works:

- Each running app is an iroh endpoint identified by an Ed25519 key.
  Connections are direct QUIC with hole punching; iroh relays are only a
  fallback transport, never a content host.
- A Commy server is a gossip topic. Every state change is a `ServerEvent`
  broadcast to the topic and applied idempotently to each member's
  replicated `ServerState`.
- Invites are self-contained `commy...` tickets carrying the topic id and
  bootstrap addresses. Newcomers request a state snapshot from any member.
  Short invite links store the ticket sealed with a key that only the link
  carries, so the account server cannot read it.
- Files, avatars, emojis, and soundboard clips move over iroh-blobs,
  addressed by blake3 hash; chat messages carry only the hash.

## Prerequisites

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

## Develop

```sh
# Run every Rust unit test (core, net, audio)
cargo test --workspace

# Run the desktop app
cd apps/desktop/src-tauri
cargo tauri dev

# Test voice, screen share, and DMs on ONE machine: run a second
# instance with its own identity and data dir, then join your own
# server with an invite. Settings also has a Mic Test loopback.
# PowerShell:
#   $env:COMMY_DATA_DIR = "$env:APPDATA\chat.commy.desktop-2"
#   cargo run -p commy-desktop

# Preview the UI alone in a browser (mock backend)
# open apps/desktop/ui/index.html

# Account server and hosting stack: see deploy/HOSTING.md
```

Before bundling installers, generate platform icons once:

```sh
cargo tauri icon apps/desktop/ui/images/commy_v2.png
```

then set `bundle.active` to `true` in `apps/desktop/src-tauri/tauri.conf.json`.

## Status

Working today:

- Domain model with replicated state, permissions, and unit tests
- Invite tickets and a versioned gossip wire protocol
- Text chat over gossip with newcomer state sync
- Voice chat: mic -> RNNoise noise cancellation -> PCM over direct
  QUIC media connections (full mesh, smaller node id dials), jitter
  buffered and mixed per peer on playback
- Screen share: primary monitor -> MJPEG at 8 fps over a direct QUIC
  stream, viewers subscribe via a LIVE badge in the member list
- File sharing: attach from chat, content-addressed via iroh-blobs,
  download fetches from the uploader
- Persistence: profile, voice settings, DMs, and per-server snapshots
  under the app data dir, written atomically and loaded at startup
- Direct messages: ack-confirmed delivery over a dedicated QUIC
  protocol, with cached contact profiles (both peers must be online)
- Server management: rename, create and delete channels, leave server,
  gated by the role permission system
- Account management: display name, custom status, presence, broadcast
  to every joined server
- Rich chat: emoji picker, inline image and gif rendering for
  attachments, paste and drag-drop upload, clickable links opened in
  the system browser, auto-embedded image links
- Screen share picker: choose any monitor or window to stream
- Volume controls: master output and mic input gain, persisted
- Server roles: create roles with colors and permissions, assign from
  server settings; member names take their highest role color
- Kick members from server settings (KICK_MEMBERS permission)
- Auto AFK: every server has an AFK voice channel; clients park
  themselves there after 5 minutes with no audible mic input, no
  keyboard or mouse use, and no active screen share
  (COMMY_AFK_SECS overrides the timeout)
- Resting (test setting): with COMMY_REST_AFTER_SECS set, an away
  client leaves every server's mesh after that many idle seconds and
  keeps only its relay link; a member who sends a message wakes it by
  node id (`commy/wake/1`, one byte, no server or message named) and it
  rejoins and catches up
- Mic test loopback and multi-instance support (COMMY_DATA_DIR) for
  single-machine voice testing
- Desktop UI shell

Voice transport notes: media connections use a dedicated `commy/media/1`
ALPN. Voice is raw PCM for now (about 96 kB/s per peer); the packet
format has room for an Opus payload, which is the planned upgrade.
Screen share is MJPEG for the same reason; a real video codec can slot
into the same `MediaPacket::Video` frame.

Next milestones:

1. Opus voice encoding and per-user volume sliders.
2. Hardware-accelerated video for screen share.
3. Soundboard and custom emoji management.
4. Direct messages between friends.
5. Device linking against the auth website (Device model exists).

Note: iroh's API moves quickly between minor versions. The workspace pins
0.28; check the iroh changelog when bumping.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the branch flow and checks.

## License

Commy is licensed under the [GNU Affero General Public License v3.0](LICENSE).
