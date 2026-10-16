# Commy

Commy is free, open source, P2P voice and text chat for communities, written in Rust. It comes with private P2P voice, P2P file sharing (isolated to your server), soundboards, emojis, images, and screen sharing with no paywall. Must have a easy noise cancellation feature toggle.  

## Project Structure

This is a Desktop App built with Rust compatible with Windows + MacOS intitially. 
Use IROH for Calls, Screen Sharing, and File Sharing.
Our Auth / Docs Website should use Postgres,Prisma,Auth.JS with Google Social Login and Email. Use a docker compose stack for Dockhand. 

## Key Files & Directories

- `Cargo.toml` - workspace root; pins shared dependency versions (iroh 0.28)
- `crates/commy-core/` - domain model: ids, users, servers, channels, messages, permissions, `ServerState` (replicated, idempotent event application)
- `crates/commy-net/` - P2P: `ServerTicket` invites, versioned gossip `Envelope` protocol, `CommyNode` iroh wrapper (gossip topics per server, blobs for files)
- `crates/commy-audio/` - voice DSP: `VoiceProcessor` with RNNoise noise cancellation toggle, mute gate, VAD (480-sample frames at 48 kHz)
- `crates/commy-media/` - device I/O: cpal mic capture and playback, `JitterBuffer`, `Mixer`, xcap screen capture to MJPEG
- `apps/desktop/src-tauri/` - Tauri 2 shell: commands in `src/lib.rs`, state in `src/state.rs`, view models in `src/view.rs`, voice/screen orchestration in `src/media_engine.rs`, disk persistence in `src/persist.rs`
- `apps/desktop/ui/` - static frontend (no build step); works in a browser with a mock backend for preview; `images/`, `sounds/` (soundboard clips in `sounds/soundboard/`), `scripts/` (`emoji.js` holds the picker data), `styles/`, `fonts/`
- Dev testing: run a second instance with `COMMY_DATA_DIR` set for P2P testing on one machine (its webview storage moves there too, `COMMY_INSTANCE_LABEL` names the window); `auth/docker-compose.dev.yml` runs a local account server with mail catcher, Redis and Prisma Studio, optional keys go in `auth/.env`; `COMMY_AFK_SECS` shortens the AFK timeout; UI edits need `cargo build` (assets embed at compile time)
- CI: "Rust checks" (`.github/workflows/rust.yml`) compiles every PR that touches Rust on Windows, macOS and Linux in a few minutes; never run "Desktop release" by hand to prove a branch compiles
- The commy.chat website (Next.js) lives in its own private repository, commy-web; `auth/` is the Better Auth account server and `deploy/` the hosting stack

## Development Guidelines

### Coding Style

Avoid emojis and M-Dashes, also don't be too verbose in code comments. Use generated assets and icon libraries. 

### Testing Approach

Create Robust Unit Tests for all new modules written, QA testing is high priority. 

Never name other chat products or their paid tiers in code, comments, docs or UI copy; describe Commy on its own terms.
Never put personal names or emails in the repo; the public identity is Commy Commander.

## Files to Avoid

- node_modules/
- .env files
- Build artifacts (dist/, build/)
- Log files

## Special Instructions

We are aiming for a complete, familiar community chat app on Windows first (servers, channels, roles, voice, screen share), it should use P2P as we won't be hosting anything other than docs and account auth. It should be seamless to install and setup. 


## Playbooks

Keep this file short — it is read before every task. Long procedures belong in
their own file, referenced here, and loaded only when the task matches:

- Cutting a release → read `.claude/playbooks/release.md` first
- <another long procedure> → read `.claude/playbooks/<name>.md` first

## Working with Claude Code

### Preferred Workflow

1. **Explore**: First understand the codebase
2. **Plan**: Discuss the approach before coding
3. **Code**: Implement with tests
4. **Commit**: Use conventional commits

### Git Workflow

- Use Claude for 90% of git operations
- Create descriptive commit messages
- Branch naming: feature/*, bugfix/*, docs/*

### Communication Style

- Ask clarifying questions before implementing
- Explain technical decisions
- Suggest improvements when relevant
- Point out potential issues proactively

---

