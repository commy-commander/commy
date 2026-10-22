# Commy in the browser

The web client runs the desktop UI (`apps/desktop/ui`) on a WebAssembly build
of the same Rust code the desktop app uses. It is an early preview: sign in,
open your servers, read text channels and send messages. Voice, video, files,
direct messages and server settings still need the desktop app.

## How it fits together

- `src/` is the backend, compiled to `wasm32-unknown-unknown`. It answers the
  same commands the desktop backend answers (`list_servers`, `get_messages`,
  `send_message` and so on) with the same JSON, using `commy-core`,
  `commy-net` and `commy-view`.
- `ui/web-bridge.js` loads before the app's scripts and stands in for the
  desktop bridge, so `apps/desktop/ui/scripts/app.js` runs unchanged.
  `ui/web-commands.js` holds the answers for desktop-only commands.
- The page must be served from the same site as the account server, because
  it signs in with that site's session cookie and talks to `/api/auth`.
- A browser cannot open UDP sockets, so every connection goes through the
  relay. The relay only forwards encrypted traffic.
- A new browser gets your server list key from one of your other devices
  ("Allow this device?") or from your recovery code. It then asks one of your
  own devices for a copy of each server; that request is signed with your
  account identity, so the account server alone cannot obtain messages.
  Missing history comes from other members through the usual catch-up.
- When the account server names a sync service (`endpoints.sync` in
  `/.well-known/commy-auth.json`, on the same site), the browser also follows
  the same end-to-end encrypted streams as the desktop app, through
  `commy-sync`'s client over the browser's WebSocket: your account log (server
  list and community keys) and each server's state and text channel streams.
  A server can then open from its owner's signed snapshot with none of your
  devices online, and messages you send are also added to the channel's
  stream. The stream keys come from your server list key and the community
  keys; the page's session is never sent to the sync service.

## Build

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --locked --version "$(cargo metadata --format-version 1 --locked | jq -r '.packages[] | select(.name == "wasm-bindgen") | .version')"
node apps/web/build.mjs
```

The bundle lands in `apps/web/dist`. Build settings:

- `COMMY_WEB_RELAYS`: a comma-separated list of relays the page may use. The
  page's content security policy allows only its own site and these relays.
  The first one is the page's relay unless the site serving it says otherwise.
  Without it, `COMMY_DEFAULT_RELAY` or `https://relay.commy.chat:8443`.
- `COMMY_WEB_BASE`: the path the page is served under, such as `/web/`.
- `COMMY_WEB_OUT`: where to write the bundle.

The page names its relay in `<meta name="commy-relays">`. A site serving one
bundle for several environments rewrites that tag's `content` to pick the
relay, as long as the relay is one the bundle was built to allow.

## Tests

```sh
cargo test -p commy-web-client
node --test "apps/web/tests/**/*.test.mjs"
```
