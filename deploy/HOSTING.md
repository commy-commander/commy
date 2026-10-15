# Hosting Commy

Chat, voice, screen sharing and files in Commy are peer to peer and never touch
a server. The only hosted pieces are accounts and the website, and this folder
runs all of them on one machine with Docker Compose:

| Service    | Image                 | Role                                                  |
|------------|-----------------------|-------------------------------------------------------|
| `caddy`    | `caddy:2.10-alpine`   | Public entry on ports 80/443, automatic HTTPS         |
| `web`      | `ghcr.io/commy-commander/commy-web` | Website and docs (Next.js, own repository) |
| `auth`     | built from `../auth`  | Better Auth server: accounts, desktop sign-in, devices |
| `postgres` | `postgres:16-alpine`  | Auth database, private network only                   |

Routing on `https://<DOMAIN>`: `/api/auth/*` and `/.well-known/commy-auth.json`
go to `auth`, everything else to `web`. `docs.<DOMAIN>` redirects to `/docs`.

## Requirements

- A Linux VPS with Docker Engine and the Compose plugin (1 GB RAM is enough)
- A domain with an A (and optionally AAAA) record pointing at the server, plus
  one for `docs.<DOMAIN>` if you want the docs redirect
- Ports 80 and 443 open (Caddy needs 80 to get certificates)

## Install

```bash
git clone https://github.com/commy-app/commy.git
cd commy/deploy
cp .env.example .env
```

Edit `.env`: set `DOMAIN`, `ACME_EMAIL`, and generate the two secrets:

```bash
openssl rand -hex 24      # POSTGRES_PASSWORD
openssl rand -base64 32   # BETTER_AUTH_SECRET
```

Then start everything:

```bash
docker compose up -d --build
docker compose logs -f auth
```

The auth server creates its tables on first boot. Open `https://<DOMAIN>/account`
to create the first account.

Dockhand (or Portainer) can deploy the same stack: point it at this repository
with `deploy/docker-compose.yml` as the compose path and paste your `.env`
values into its environment editor.

## Sign-in providers

Email and password works with no extra setup. The rest are optional:

- **Google**: in https://console.cloud.google.com, configure the OAuth consent
  screen (External, scopes `openid`, `email`, `profile`), then create an
  OAuth client of type Web application with authorised JavaScript origin
  `https://<DOMAIN>` and redirect URI `https://<DOMAIN>/api/auth/callback/google`.
  Set `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET`.
- **Twitch**: register an application at https://dev.twitch.tv/console/apps
  (category Website Integration, client type Confidential) with OAuth Redirect
  URL `https://<DOMAIN>/api/auth/callback/twitch`, then set `TWITCH_CLIENT_ID`
  and `TWITCH_CLIENT_SECRET`. Commy asks Twitch for the account email; a
  Twitch sign-in joins an existing Commy account only when Twitch reports that
  email as verified.
- **Email** (verification, password reset, magic links): set `SMTP_HOST`,
  `SMTP_PORT` (587 for STARTTLS, 465 for TLS), `SMTP_USER`, `SMTP_PASSWORD`
  and `EMAIL_FROM`, or a single `SMTP_URL`. Many providers want an
  app-specific SMTP token as the password, and `EMAIL_FROM` must be an address
  that user may send from. With SMTP set, new email accounts must verify their
  address before signing in; a sign-in attempt with an unverified address sends
  a fresh link, and the sign-in page offers a resend. Verification links last
  24 hours, reset links 1 hour, and a reset signs out every other session.
  Without SMTP, accounts work unverified and password reset is unavailable (the
  emails are dropped, never logged).

Every redirect URI follows the site's public URL, so a staging copy on
`staging.<DOMAIN>` needs its own redirect URIs added to the same Google client
and Twitch application (both accept several).
- **Bot check**: set `TURNSTILE_SITE_KEY` and `TURNSTILE_SECRET_KEY` from a
  Cloudflare Turnstile widget for your domain to put a bot check on sign-up,
  sign-in, magic links and password reset. Recommended for any public instance.

See [GIF search and Twitch](#gif-search-and-twitch) below for GIFs, game
search and the Twitch app they share.

Set `ALLOW_SIGNUP=false` to close registration on a private instance.
Run `docker compose up -d` after any `.env` change.

## GIF search and Twitch

Signed in apps search GIFs (KLIPY) and games (IGDB, which uses a Twitch
application) through your account server. The keys stay in your `.env`: apps
only ever get results, so your users need no keys of their own and nothing
worth extracting ships in the app. The GIF files and game covers load through
the server as well (`/api/auth/commy/media`), so apps talk only to their
account server and KLIPY and IGDB never see your users' addresses. That route
needs a signed in app and fetches only from `static.klipy.com` and
`images.igdb.com`, so it is not an open proxy; it holds up to 64 MB of recent
images in memory. Budget for the image traffic: a chat GIF is usually 1 to 3 MB. The discovery document lists what is set up
under `features`, and the app hides a search your server does not offer
(GIF button, game suggestions) unless the user adds a key of their own.

1. **Twitch app** (sign-in and game search). Sign in at
   https://dev.twitch.tv/console/apps (Twitch asks you to turn on two-factor
   sign-in first) and choose Register Your Application:
   - Name: anything unique, such as your instance name
   - OAuth Redirect URLs: `https://<DOMAIN>/api/auth/callback/twitch`
     (add one per hostname, for example a staging copy, with Add)
   - Category: Website Integration
   - Client Type: Confidential

   Open Manage, copy the Client ID, press New Secret and copy the secret.
2. **KLIPY key** (GIFs). Sign in at https://partner.klipy.com/api-keys, choose
   Add Platform, then Create API Key, and copy it. Test keys are rate limited;
   use Request Production in the key menu before opening to the public.
3. Fill in `deploy/.env`:

   ```sh
   TWITCH_CLIENT_ID=<client id>
   TWITCH_CLIENT_SECRET=<client secret>
   KLIPY_API_KEY=<klipy key>
   ```

   Leave `IGDB_CLIENT_ID` and `IGDB_CLIENT_SECRET` blank: game search reuses
   the Twitch app. Set them only to use a separate Twitch app for IGDB, for
   example to offer game search without Twitch sign-in.
4. Run `docker compose up -d`, then check the start of the auth log with
   `docker compose logs auth | head`. It prints one line such as
   `[auth] google=off twitch=on email=on turnstile=on gifs=on games=on (twitch app) signup=on`
   and a warning naming any half-filled pair, such as a client id without its
   secret. `https://<DOMAIN>/.well-known/commy-auth.json` should show
   `"features":{"gifs":true,"games":true}`.

Apps already signed in pick up a change the next time they start. Leaving a
value blank turns that part off cleanly: the search answers 404 ("not set up on
this account server") and the app hides it. Search terms are forwarded to
KLIPY or IGDB and never logged or stored; failed upstream calls are logged
with the key masked.

## Pointing the desktop app at your server

The app reads `https://<DOMAIN>/.well-known/commy-auth.json`, which lists the
API base, the enabled providers and the device sign-in endpoints. Set the
account server in the app to `https://<DOMAIN>` (or launch it with
`COMMY_AUTH_URL=https://<DOMAIN>`).

Sign-in uses the OAuth device flow (RFC 8628): the app shows a short code and
opens `https://<DOMAIN>/device`, you approve it in the browser, and the app
receives a bearer token. The approval page warns when the request came from a
different network than the browser approving it, the usual sign of someone
phishing an approval.

To link its iroh node id, the app asks `POST /api/auth/commy/devices/challenge`
for a challenge, signs the returned `message` with its node secret key
(ed25519), and sends `nodeId`, `challenge` and the hex `signature` to
`POST /api/auth/commy/devices/register`. Only the holder of a node key can link
that node id, and the message names the server URL, so a signature for one
server cannot be replayed on another.

## Invite links

The app turns a server invite into a short link on its account server,
`https://<DOMAIN>/i/<CODE>#<KEY>`. The page there shows the server name and
opens the app with `commy://i/<DOMAIN>/<CODE>#<KEY>`, so the code is always
resolved on the server that issued it. People can also paste the link into Add
a Server.

The app seals the invite with `<KEY>` (AES-256-GCM) before sending it, and the
key only travels in the link's `#` fragment, which browsers and the app never
send to a server. The account server stores the code, the sealed invite and
its expiry, so it cannot tell which Commy server an invite opens, what it is
called, or who shared it. The invite page opens the sealed invite in the
visitor's browser to show the server name. Links made by older app versions
have no key; they keep working until they expire, and the server no longer
accepts new ones.

- Anonymous links expire after 7 days, links from signed in apps after up to
  30. Creating links is limited to 30 per hour per IP; set
  `INVITES_REQUIRE_ACCOUNT=true` to only allow signed in apps.
- The app warns before joining from a link whose server is not the account
  server it is set to, since anyone can run a Commy server.
- Inside the seal is what any Commy invite contains: the server id and name,
  and the network addresses of the member who created it. Only people with the
  link can read it.

## iroh relay

The stack runs its own [iroh](https://iroh.computer) relay at
`https://relay.<DOMAIN>:8443`. Apps use it first and keep n0's public relays as
a fallback. Relays help peers find each other and carry traffic only when a
direct connection is impossible, always end to end encrypted.

- **DNS:** an A record for `relay.<DOMAIN>`, DNS only (not proxied) in
  Cloudflare. Open `8443/tcp` (relay over HTTPS, certificate via port 80) and
  `3478/udp` (STUN).
- **Bandwidth budget:** `relay-meter` adds up relayed bytes each month (UTC).
  At `RELAY_WARN_PERCENT` of `RELAY_MONTHLY_LIMIT_GB` it posts to
  `ALERT_WEBHOOK_URL`; at `RELAY_PAUSE_PERCENT` it stops the relay until the
  1st, and apps move to the fallback relays on their own. Set the limit below
  your VPS plan's allowance to leave room for the website.
- **Usage so far:** `docker compose exec relay-meter cat /data/usage.json`.
- **Pause or resume by hand:** `docker compose exec relay-meter touch /flags/relay-paused`
  stops the relay until you delete that file; the meter only clears pauses it set.
- **Several environments on one server:** run one shared relay with
  `docker-compose.relay.yml` (see the comment at its top); the per
  environment stacks leave the relay out.
- **Apps:** `COMMY_RELAY_URLS=https://relay.example.org:8443` points a build at
  another relay, `none` uses only n0's.

The relay must run the same iroh version as the app (`IROH_NET_VERSION` in
`relay/Dockerfile`), so the two are upgraded together.

## What the operator can see

Commy messages, voice, video, screen shares and files are peer to peer and
never reach this stack. The auth database holds:

- accounts: email, display name, avatar URL, password hash (scrypt)
- Google sign-ins: provider account id, with OAuth tokens encrypted at rest
- sessions: token and expiry only (IP address and user agent are not stored)
- linked devices: iroh node id, device name, first and last link time
- invite links: the code, the sealed invite (unreadable without the link),
  creator account if the app was signed in, expiry
- the relay: IP addresses and node ids of connected apps while they are
  connected, and monthly byte totals; relayed traffic is encrypted end to end
- short-lived rows for verification, device sign-in codes, and a keyed hash
  of the requesting network (deleted after 15 minutes, never the raw IP)

IP addresses are used in memory for rate limiting only. The single server
Caddyfile writes no access logs. The shared edge proxy (`edge/`) writes JSON
access logs to `/var/log/commy-caddy`, except for invite pages and invite
lookups, which are never logged. The auth server logs no emails, links or
tokens.

## Abuse controls

- Rate limits per IP: sign-up 5 per 10 minutes, sign-in 10 per minute, magic
  links and password resets 5 per 10 minutes, device codes 10 per minute.
- Optional Turnstile bot check and `ALLOW_SIGNUP=false` (see above).
- Device codes expire after 10 minutes and must be approved by a signed-in user.
- Behind another proxy or CDN (for example Cloudflare), rate limits only see
  the real client IP if that proxy's address forwarding reaches Caddy.

## Operations

```bash
docker compose ps                                   # health of every service
git pull && docker compose up -d --build         # update to the latest release
docker compose exec postgres pg_dump -U commy commy_auth > backup.sql
docker compose exec -T postgres psql -U commy commy_auth < backup.sql
```

To have the server update itself whenever GitHub publishes new images, or to run
dev, staging and prod side by side, see [DEPLOYING.md](DEPLOYING.md).

Volumes: `pgdata` holds accounts, `caddy_data` holds TLS certificates. Back up
`pgdata` and your `.env`. Rotating `BETTER_AUTH_SECRET` signs everyone out.

## Trying it locally

```bash
# deploy/.env with DOMAIN=localhost, ACME_EMAIL=you@example.org, both secrets, plus:
#   PUBLIC_URL=https://localhost:8443
#   HTTP_PORT=8080
#   HTTPS_PORT=8443
docker compose up -d --build
```

Caddy serves `localhost` with its own local certificate authority, so the
browser will warn until you trust it. Auth server unit tests run with
`cd ../auth && npm install && npm test`.
