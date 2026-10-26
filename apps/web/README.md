# Commy in the browser

The web client runs the desktop UI (`apps/desktop/ui`) on a WebAssembly build
of the same Rust code the desktop app uses. It is a preview: sign in, open
your servers, chat in text channels, send files and voice messages, use direct
messages and group DMs, call people, join voice channels, create servers,
arrange their channels, make and open invite links, manage roles, moderate
members, set the rules, delete or hand over a server, and edit your profile
(name, status, About Me, avatar, banner, pronouns, a status that runs out
and who sees your profile details), set server, channel and role pictures,
add custom emoji, stickers, emoticons and sounds, play the soundboard, plan
events and RSVP, post in forums, run stage channels (speakers, raised hands,
server mute and deafen), set voice channel statuses, set up onboarding
questions, AutoMod and the Activities switch, export a server as a template,
report members to the server owner, set message reminders, and browse or list
servers in Discover. Links from another
account server, video and screen sharing still need the desktop app.

Pictures (avatars, server icons and banners, custom emoji) come from the sync
service's encrypted picture copies when it has them, and otherwise straight
from the member who shared them. A file or voice message sent from the browser
is served by that tab, so others can open it while the tab stays open.

Message, DM and friend notices use the browser's own notifications, following
the same notification settings as the desktop. The browser asks once for
permission, on your first click after your servers open, and shows notices
only while the tab is in the background. Clicking one opens that channel or DM.

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
  ("Allow this device?", with the same 6-digit code on both screens) or from
  your recovery code. It asks through the sync service when the account server
  names one, and straight over the relay only when it does not. It then asks
  one of your own devices for a copy of each server; that request is signed with your
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
