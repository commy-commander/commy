# How Commy works

## Where things live

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

## How it fits together

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
  [deploy/HOSTING.md](../deploy/HOSTING.md#what-the-operator-can-see).
