# Hosting Commy

Chat, voice, screen sharing and files in Commy are peer to peer and never touch
a server. The only hosted pieces are accounts and the website, and this folder
runs all of them on one machine with Docker Compose:

| Service    | Image                 | Role                                                  |
|------------|-----------------------|-------------------------------------------------------|
| `caddy`    | `caddy:2.11-alpine`   | Public entry on ports 80/443, automatic HTTPS         |
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
git clone https://github.com/commy-commander/commy.git
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
  `EMAIL_REPLY_TO` optionally sends replies to another mailbox. To keep these
  emails out of spam, send from your own domain through your mail provider and
  publish the SPF, DKIM and DMARC records it gives you for that domain.

Every redirect URI follows the site's public URL, so a staging copy on
`staging.<DOMAIN>` needs its own redirect URIs added to the same Google client
and Twitch application (both accept several).
- **Bot check**: set `TURNSTILE_SITE_KEY` and `TURNSTILE_SECRET_KEY` from a
  Cloudflare Turnstile widget for your domain to put a bot check on sign-up,
  sign-in, magic links and password reset. Recommended for any public instance.

See [GIF search and Twitch](#gif-search-and-twitch) below for GIFs, game
search and the Twitch app they share.

Set `ALLOW_SIGNUP=false` to close registration on a private instance, or set
`ACCESS_CODE` to let in only people you give a code to: the website asks for it
before showing any page except the privacy policy, and the auth server refuses
new accounts from anyone who has not entered it. Each browser asks once, then
remembers it for 90 days.

The website serves the web app at `/web`. It uses the production relay unless
`COMMY_RELAY_URLS` names another one; a staging site sets it to its staging
relay so browsers meet the test builds there.
Run `docker compose up -d` after any `.env` change.

## GIF search and Twitch

Signed in apps search GIFs and stickers (KLIPY) and games (IGDB, which uses a Twitch
application) through your account server. The keys stay in your `.env`: apps
only ever get results, so your users need no keys of their own and nothing
worth extracting ships in the app. The GIF and sticker files and game covers load through
the server as well (`/api/auth/commy/media`), so apps talk only to their
account server and KLIPY and IGDB never see your users' addresses. That route
needs a signed in app and fetches only from `static.klipy.com`,
`images.igdb.com`, the GIPHY image hosts (`media*.giphy.com`,
`i.giphy.com`) and emoji.gg's emoji images (`cdn3.emoji.gg/emojis/`), so it
is not an open proxy; it holds up to 64 MB of recent images in memory. Budget
for the image traffic: a chat GIF is usually 1 to 3 MB. The discovery document lists what is set up
under `features`, and the app hides a search your server does not offer
(GIF button, game suggestions) unless the user adds a key of their own.

Emoji search (`/api/auth/commy/emojis/search`, used when adding server emojis
and stickers) needs no key. The server downloads emoji.gg's public catalog
(about 2 MB) on the first search, keeps it in memory for six hours, leaves out
the adult category and searches it locally, so emoji.gg sees only your
server, never your users.

1. **Twitch app** (sign-in and game search). Sign in at
   https://dev.twitch.tv/console/apps (Twitch asks you to turn on two-factor
   sign-in first) and choose Register Your Application:
   - Name: anything unique, such as your instance name
   - OAuth Redirect URLs: `https://<DOMAIN>/api/auth/callback/twitch`
     (add one per hostname, for example a staging copy, with Add)
   - Category: Website Integration
   - Client Type: Confidential

   Open Manage, copy the Client ID, press New Secret and copy the secret.
2. **KLIPY key** (GIFs and stickers). Sign in at https://partner.klipy.com/api-keys, choose
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
   `[auth] google=off twitch=on email=on turnstile=on gifs=on games=on (twitch app) doorbell=on redis=on signup=on`
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

- Whoever creates a link picks when it expires: in 1 hour, 1 day, 7 days or
  30 days. On a server its owner marked public, the app also offers links
  that never expire; the account server keeps those, only for signed in
  apps and with the account that made each one,
  up to `INVITES_MAX_PERMANENT` of them (20000 by default). The app seals the
  expiry inside the invite as well, so a row kept past its expiry still will
  not open. Creating links is limited to 30 per hour per IP; set
  `INVITES_REQUIRE_ACCOUNT=true` to only allow signed in apps.
- Someone who can manage a server can pick a custom name instead of a random
  code, like `https://<DOMAIN>/i/pixel-pioneers#<KEY>`. Custom names need a
  signed in app, are first come first served, and each account holds up to 10.
  They still carry a short key, so the account server cannot tell which server
  a name opens.
- A server can set an invite background. The app seals the image with the
  link's key and sends it with the link (only from a signed in app), so the
  account server stores it as unreadable bytes next to the code and drops it
  when the link expires or is revoked.
- Each link also gets a revoke secret that only the server's members hold; the
  account server keeps a SHA-256 hash of it. Server Settings > Invites lists a
  server's active links, and its managers can revoke any of them from there.
- The app warns before joining from a link whose server is not the account
  server it is set to, since anyone can run a Commy server.
- Inside the seal is what any Commy invite contains: the server id and name,
  and the network addresses of the member who created it. Only people with the
  link can read it.

## Message doorbell

Commy messages travel only between members' devices, so a message sent while
its recipient is offline waits on the devices that hold it (the sender's, and
those of members who were online when it was sent) until one of them and the
recipient are online at the same time. The doorbell closes that gap without carrying any message:
the sender's app rings the recipient's doorbell on the recipient's account
server. When the recipient's app is running it hears the ring, shows the
messages as on their way and fetches them from its peers directly once one of
them has the messages.

Each app picks a random key for every Commy server it is in and shares it only
with that server's members, peer to peer. From the key both sides derive a
token that changes every day and a blinded id for each message, so the account
server only ever sees random values:

- `POST /api/auth/commy/doorbell/ring` with `{ token, ids }` stores up to 50
  blinded ids for one token. An empty list is a hello: it wakes an app that is
  listening right now and is never stored.
- `POST /api/auth/commy/doorbell/take` with `{ token, wait }` returns and
  deletes the ids stored for one token, waiting up to 25 seconds for a ring.
  Apps keep one take open per Commy server.

Neither needs a session, and every request carries exactly one token, so no
request ties two tokens together.

- **What it stores:** random tokens and blinded ids with the time they
  arrived, at most 1000 per token and 200,000 in all (rings beyond that are
  refused until there is room). A take deletes what it returns, and an hourly
  sweep deletes anything older than 14 days.
- **Where:** in the stack's Redis (`REDIS_URL`, set by `docker-compose.yml`),
  as one sorted set per token. Rings are taken in one atomic step and fanned
  out over Redis pub/sub, so a take waiting on one auth instance wakes from a
  ring on another. Without `REDIS_URL` the auth server keeps rings in its
  `doorbellRing` table instead, which it creates when it starts; see
  [Redis](#redis) below.
- **Setting it up:** nothing to do. It is on by default and `deploy/Caddyfile`
  keeps no access log. If you put your own proxy in front, leave
  `/api/auth/commy/doorbell/*` out of its access log, as `deploy/edge/Caddyfile`
  does.
- **What it never sees:** Commy server ids, accounts or sessions, message
  content, who sent or receives a message, or who is in which server. It
  stores no IP address or user agent; addresses are used in memory for rate
  limits only (300 rings and 600 takes per minute per IP), and the edge proxy
  never logs doorbell calls.
- **Turning it off:** set `DOORBELL=false` in `.env` and run
  `docker compose up -d`. The startup log then shows `doorbell=off`, the
  discovery document shows `"doorbell":false` under `features`, the endpoints
  answer 404, and messages for apps signed in to this server arrive only when
  sender and recipient are online together. Rings already stored stay until
  you delete them:
  `docker compose exec redis redis-cli --scan --pattern 'doorbell:*' | xargs docker compose exec -T redis redis-cli del`
  (or, without Redis,
  `docker compose exec postgres psql -U commy commy_auth -c 'delete from "doorbellRing"'`).

## Redis

The stack runs a small Redis (`redis:8-alpine`, 128 MB cap, reachable only
from the auth server) for the two kinds of state that are short-lived and hot:

- **Doorbell rings:** random tokens and blinded ids with their arrival time,
  as described above. Expiry: taken by an app or swept after 14 days.
- **Rate-limit counters:** one counter per caller and endpoint. Before a
  counter is written its key (IP address and path) is hashed with
  `BETTER_AUTH_SECRET`, so Redis holds a keyed hash and a number, never an
  address, and the counter expires with its window (at most one hour).
  Counters are shared by every auth instance and survive a restart.

Everything durable (accounts, sessions, linked devices, invite links, the
sealed server lists) stays in Postgres. Redis snapshots to its `redisdata`
volume once a minute, so rings survive a restart; losing the volume loses at
most undelivered rings, which the apps send again when both sides are online.

**Redis is optional.** To run without it, set `REDIS_URL=` (empty) in `.env`
and remove the `redis` service and the auth service's `depends_on: redis`
entry from `docker-compose.yml`. Rings then go to the `doorbellRing` table and
rate limits are counted in each auth process's memory, as before. The startup
log says which: `redis=on` or `redis=off`. The auth server does not wait for
Redis to start. If Redis is configured but unreachable, sign-in keeps working;
doorbell calls fail until it is back, rate limits are off meanwhile (the edge
proxy keeps its own), and `/healthz` reports `"redis":false`.

## Sync service

The stack can run a sync service (`sync`, from `crates/commy-sync-server`) at
`wss://<DOMAIN>/sync`. It keeps ordered logs of entries that the apps seal
end to end before sending, and hands each new entry to every device following
that log. It cannot open an entry, and it never learns which account, Commy
server or channel a log belongs to.

- **How apps find it:** set `SYNC_URL=wss://<DOMAIN>/sync` in `.env`. The
  discovery document then lists it under `endpoints.sync` and shows
  `"sync":true` under `features`. Without `SYNC_URL` apps do not use it.
- **How a device gets in:** every log is named by a stream id that the apps
  derive, together with a write key and a read token, from a secret only its
  members hold. The service checks that the stream id matches the write key and
  read token it is shown, and that each new entry is signed by that write key.
  It takes no account, session or cookie.
- **What the apps keep there:** one log per account, sealed with the account's
  server list key, for the server list, read positions, saved messages and
  notification settings, so every signed-in device follows changes within
  seconds; and one log per pair of people per month for direct messages, so a
  DM reaches all of each person's devices. The apps still sync the server list
  through the account server too, so older versions keep working.
- **What it stores:** in the `commy_sync` database (created on first start in
  the stack's Postgres): for each log, its random stream id, the number of
  entries so far and the time of the last one; for each entry, its number,
  arrival time, a random retry nonce and the sealed bytes, padded to one of a
  few fixed sizes so lengths say little.
- **How long:** `SYNC_RETENTION_DAYS` (90 by default, `0` keeps everything).
  An hourly sweep deletes older entries, and a log with no entries left is
  forgotten.
- **What it never sees:** what an entry says, who wrote it, which account or
  Commy server a log belongs to, or which logs one person follows. It stores no
  IP address. Addresses are used in memory for rate limits only, under a keyed
  hash with a secret that changes on every restart. The edge proxy never logs
  `/sync`.
- **Waking phones (optional):** with `FCM_SERVICE_ACCOUNT` set to a Firebase
  service account key (the JSON, or its base64), the service can wake the
  Android app with an empty push when something new arrives. A phone registers
  short-lived wake tags (`POST /sync/wake/register`), each derived from a log's
  secret, the person it wakes and the day, so a tag cannot be tied to a stream
  id and changes daily. A device that writes to a log rings its members' tags
  (`POST /sync/wake/ring`) in a separate request. The service then sends one
  push with no content to each phone registered on those tags, at most one
  every 2 minutes per phone. It stores each tag with the push token and the
  last day it is valid (2 days ahead at most), and drops tokens Google reports
  as gone. It keeps no record of who rang, but an operator watching live
  traffic could pair the address that rang with the phone it woke. Without the
  key, `GET /sync/wake` answers 404 and apps do not register.
- **Setting it up:** `docker compose up -d sync`, then add `SYNC_URL` and
  recreate auth: `docker compose up -d --no-deps auth`. The service answers
  `GET /sync/healthz`. It logs only totals (connections, live logs, opens and
  appends per 10 minutes, and wake-ups when they are on).

## iroh relay

The stack runs its own [iroh](https://iroh.computer) relay at
`https://relay.<DOMAIN>:8443`. Apps use only this relay while it answers, which
keeps them reachable from the web app. If it stays out of reach for 30 seconds,
apps add n0's public relays as a fallback. Relays help peers find each other and carry traffic only when a
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
  `docker-compose.relay.yml`; the per environment stacks leave the relay out.
  [DEPLOYING.md](DEPLOYING.md) has the commands.
- **Apps:** `COMMY_RELAY_URLS=https://relay.example.org:8443` points a build at
  another relay, `none` uses only n0's.

The relay must run the same iroh version as the app (`IROH_RELAY_VERSION` in
`relay/Dockerfile`), so the two are upgraded together.

## What the operator can see

Commy messages, voice, video, screen shares and files are peer to peer and
never reach this stack. The auth database holds:

- accounts: email, display name, avatar URL, password hash (scrypt)
- Google sign-ins: provider account id, with OAuth tokens encrypted at rest
- sessions: token and expiry only (IP address and user agent are not stored)
- linked devices: iroh node id, first and last link time (no device or
  computer name; apps no longer send one and older names are dropped at startup)
- invite links: the code, the sealed invite (unreadable without the link),
  creator account if the app was signed in, expiry, a hash of its revoke
  secret, and the sealed invite background if the server set one
- server directory listings: the invite code, server name, description,
  category, a rounded member count, and the account that listed it. A listing
  also carries its invite link's key so anyone browsing can join, which means
  the operator can open that one invite too
- each account's sealed server list (`vaultItem`): bytes sealed with a key
  only that account's devices hold, plus a version and the last change time.
  The operator can see its size, not which servers are in it
- the relay: IP addresses and node ids of connected apps while they are
  connected, and monthly byte totals; relayed traffic is encrypted end to end
- short-lived rows for verification, device sign-in codes, and a keyed hash
  of the requesting network (deleted after 15 minutes, never the raw IP)

Emails, display names, avatar URLs, session tokens, provider account ids and
ID tokens, linked node ids, passkey names, device sign-in codes and
verification values are encrypted in the database with AES-256-GCM under a
key derived from `BETTER_AUTH_SECRET`. Columns the server looks rows up by
(email, session token, provider account id, node id, device code) use a
deterministic nonce, so equal values encrypt to the same bytes and nothing
else about them shows. Email links (sign-in, password reset, two-step
challenges) are stored only as hashes. A database dump or a copy of `pgdata`
without the `.env` therefore holds no readable account data. The operator
who holds the `.env` can still read it all. Rows written by an older auth
server stay readable to the new one and are encrypted ten minutes after it
starts, once the container it replaced has stopped.

The sync service database (`commy_sync`) holds only random stream ids, entry
numbers, times, retry nonces and sealed, padded entries, plus wake tags with
push tokens when wake-ups are on. The operator can see how many logs exist and
how busy each one is, not whose they are or what they say. With wake-ups on,
the operator can also see how many tags a phone has registered and when they
are rung, not which logs or people they stand for.

Redis holds the message doorbell (random tokens and blinded message ids with
the time they arrived, deleted when an app takes them or after 14 days) and
rate-limit counters under keyed hashes of IP address and path, which expire
with their window. Without Redis the doorbell lives in the database and the
counters in memory. No IP address is written anywhere. The single server
Caddyfile writes no access logs. The shared edge proxy (`edge/`) writes JSON
access logs to `/var/log/commy-caddy`, except for invite pages, invite
lookups, doorbell calls and the sync service, which are never logged. The auth server logs no
emails, links or tokens.

## Abuse controls

- Rate limits per IP: sign-up 5 per 10 minutes, sign-in 10 per minute, magic
  links and password resets 5 per 10 minutes, device codes 10 per minute,
  doorbell rings 300 and takes 600 per minute. Counted in Redis when the
  stack has one, so they hold across restarts and auth instances.
- Sync service, per IP: 256 open connections, 5 new entries a second (bursts
  of 60) and 64 MB of entries a day (`SYNC_CONNECTIONS_PER_ADDRESS`,
  `SYNC_DAILY_BYTES_PER_ADDRESS`); per log, 20 entries a second; 16 logs per
  connection. Entries must be padded and at most 64 KB. Wake registrations
  and rings share 1 a second per IP (bursts of 120), 16 phones per tag and 8
  tags per ring.
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
docker compose exec postgres pg_dump -U commy commy_sync > sync-backup.sql
```

To have the server update itself whenever GitHub publishes new images, or to run
dev, staging and prod side by side, see [DEPLOYING.md](DEPLOYING.md).

Volumes: `pgdata` holds accounts, `caddy_data` holds TLS certificates. Back up
`pgdata` and your `.env` separately: a backup is only readable together with
`BETTER_AUTH_SECRET`, so losing the secret loses every account.

To change `BETTER_AUTH_SECRET` or go back to an auth image from before
encryption at rest, decrypt the rows first with the current secret, then
switch. The new server encrypts them again under its own secret when it
starts. Changing the secret still signs everyone out and resets two-step
sign-in.

```bash
docker compose stop auth
docker compose run --rm --no-deps auth node src/open-sealed-rows.js
```

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

To work on the auth server itself, `docker compose -f auth/docker-compose.dev.yml up -d --build`
(from the repository root) runs it with Postgres, Redis, a mail catcher and
Prisma Studio, all reachable from this machine only: the auth server on
`http://localhost:3001` (start the desktop app with
`COMMY_AUTH_URL=http://localhost:3001`), verification and sign-in mail on
`http://localhost:8025`, the tables on `http://localhost:5555` and Redis on
port 6380.
