# Deploying updates

GitHub builds the `web` and `auth` images and publishes them to GHCR. A small
agent container on the server watches one image tag per environment and swaps in
new images without downtime. GitHub never connects to the server, so there are no
SSH keys or server secrets stored in the repository.

| Environment | Image tag  | Updated by                                         | URL (example)                |
|-------------|------------|----------------------------------------------------|------------------------------|
| dev         | `:dev`     | every merge to `main` (website: in commy-web; auth: here, touching `auth/` or `deploy/`) | `https://dev.commy.chat`     |
| staging     | `:staging` | pushing a release tag like `v0.2.0`                | `https://staging.commy.chat` |
| prod        | `:prod`    | running **Promote images** and approving it        | `https://commy.chat`         |

Every build is also kept as `:sha-<commit>` and every release as `:<version>`;
promoting a version again is how rollback works.

## How an update reaches the server

1. `.github/workflows/images.yml` runs the agent tests and brings the whole
   three-environment stack up from `setup-vps.sh` on a CI machine while it builds
   the image. The image is pushed untagged and only gets its `dev`, `sha-` or
   release tags once every test has passed.
2. The agent (`agent/commy-agent.sh`, enabled by `docker-compose.agent.yml`)
   runs `docker compose pull` every minute (every 10 seconds on dev, set by
   `AGENT_INTERVAL` in that environment's `.env`).
3. When `auth` or `web` has a new image, the agent starts a second container on
   the new image next to the running one. Caddy sends traffic to both while they
   overlap and retries for up to 15 seconds if a connection fails, so visitors
   never see an error page.
4. Once the new container passes its health check the old one is stopped. If it
   never becomes healthy, the new container is removed instead, the old one keeps
   serving, and that image is skipped until a newer one is published.
   `postgres`, `redis` and `caddy` are never touched.

Watch it with `docker compose logs -f agent`.

## Setting up the server

On an Ubuntu server with Docker installed (Hostinger's Docker template works),
open a terminal (Hostinger: hPanel > VPS > **Browser terminal**) and run:

```bash
curl -fsSL https://raw.githubusercontent.com/commy-commander/commy/main/deploy/setup-vps.sh | sudo sh -s -- commy.chat you@example.org
```

This creates `/srv/commy-prod`, `/srv/commy-staging` and `/srv/commy-dev`, each a
checkout with its own `deploy/.env`, generated secrets and its own database, starts
the shared Caddy from `deploy/edge`, and starts each environment's database and
agent. The agents bring `auth` and `web` up as soon as their image tag exists, so
an environment appears on its own after its first build or promotion. Running the
script again keeps existing checkouts and `.env` files. Pass a third argument
such as `"prod"` to set up fewer environments.

Afterwards, optional settings (Google and Twitch sign-in, SMTP, Turnstile, GIF and
game search keys) go in each
`/srv/commy-<env>/deploy/.env` as described in [HOSTING.md](HOSTING.md), followed
by `docker compose up -d` in that folder. Google OAuth needs each domain's
`/api/auth/callback/google` added as a redirect URI.

### What the pieces are

- `edge/` is the only thing listening on ports 80, 443 and 8443. It routes each domain
  to that environment's containers by network alias (`commy-staging-web`,
  `commy-staging-auth`) and adds `X-Robots-Tag: noindex` on staging and dev.
  Port 8443 serves the iroh relays: `relay.<domain>` for release builds (the
  `commy-relay` project, from the prod checkout, address discovery on 7842/udp) and
  `relay-staging.<domain>` for test builds (the `commy-relay-staging` project,
  from the staging checkout, address discovery on 7843/udp). Both are DNS-only records, and
  `setup-vps.sh` starts both. Test builds never use the production relay or
  account server, and release builds never use staging.
  Sites that belong to one server only go in untracked `edge/sites.d/*.caddy` files
  (the edge imports them and runs fine when there are none), and networks the edge
  must also join go in an untracked `edge/docker-compose.override.yml`.
- `docker-compose.env.yml` turns off the per-stack Caddy and joins `web` and
  `auth` to that environment's own Docker network (`commy-prod`, `commy-staging`
  or `commy-dev`, named after `COMPOSE_PROJECT_NAME`) under those aliases. Only
  the edge Caddy is on all of them, plus `commy-relay` for the relays, so a
  container in one environment cannot reach another environment's containers.
  `setup-vps.sh` creates the four networks. Each environment's Postgres and
  Redis stay on its own private network.
- `docker-compose.agent.yml` adds the agent. It mounts the checkout read only and
  the Docker socket. Its image is built from `agent/Dockerfile` (the Docker CLI
  plus cosign).

To start or rebuild the relays by hand, run this in `/srv/commy-prod/deploy`:

```bash
docker compose -p commy-relay -f docker-compose.yml -f docker-compose.relay.yml up -d --build relay relay-proxy relay-meter
```

and this in `/srv/commy-staging/deploy` for the test build relay, which has its
own address discovery port, IPv6 subnet, image tag and a smaller monthly budget:

```bash
RELAY_PROXY_ALIAS=commy-relay-staging-proxy RELAY_DOMAIN=relay-staging.commy.chat RELAY_QAD_PORT=7843 RELAY_MONTHLY_LIMIT_GB=500 \
RELAY_IMAGE=commy-relay:staging-iroh-relay-1.4.1 RELAY_IPV6_SUBNET=fd99:6513:f442::/64 \
  docker compose -p commy-relay-staging -f docker-compose.yml -f docker-compose.relay.yml up -d --build relay relay-proxy relay-meter
```

The staging relay builds from the staging checkout under its own tag, so a
staging rebuild never replaces the image the production relay starts from.

#### Relays over IPv6

`docker-compose.relay.yml` gives each relay's network IPv6 (Docker 27 or newer),
so Docker forwards the address discovery port over IPv6 straight to the relay
and it sees each app's real IPv6 address. Each relay project needs its own
private subnet: `RELAY_IPV6_SUBNET` defaults to `fd99:6513:f442:1::/64` for
production, and staging uses `fd99:6513:f442::/64`. Apps on IPv6-only networks
also need:

- an AAAA record for each relay name, DNS only like the A record;
- the firewall to allow `8443/tcp` and each address discovery port over IPv6
  (ufw applies its rules to both when `IPV6=yes` in `/etc/default/ufw`).

Relay networks made before this have no IPv6. Recreate one by stopping and
removing that project's relay services, `docker network rm` its `_edge`
network, then running its command above again.

#### Only commy.chat apps on our relays

Each relay can ask its account servers whether a connecting app is a device
someone signed in with, and refuse everything else (other iroh apps, other
servers' users). Refused apps move to the public relays at once, so nothing
breaks, but our bandwidth only carries our own users. The relay asks
`relay-access` (in `relay-tools/`), which asks auth at `/internal/relay-access`
with a shared token. That path is not routed by Caddy. If auth does not answer,
the app is let in, so an auth restart never cuts voice off.

To turn it on, put the same `RELAY_ACCESS_TOKEN` (`openssl rand -hex 32`) in
each `.env` (prod, staging and dev; staging and dev share one, since the test relay asks both). Auth reads it after `docker compose up -d auth`. Then
start the relay projects with the access checker. Production asks prod auth:

```bash
cd /srv/commy-prod/deploy
RELAY_ACCESS_URL=http://relay-access:3342/ RELAY_ACCESS_MODE=watch \
RELAY_ACCOUNTS_NETWORK=commy-prod RELAY_ACCESS_AUTH_URLS=http://commy-prod-auth:3001/internal/relay-access \
  docker compose -p commy-relay -f docker-compose.yml -f docker-compose.relay.yml up -d --build relay relay-access
```

The test build relay asks staging and dev auth, since both use it:

```bash
cd /srv/commy-staging/deploy
RELAY_PROXY_ALIAS=commy-relay-staging-proxy RELAY_DOMAIN=relay-staging.commy.chat RELAY_QAD_PORT=7843 RELAY_MONTHLY_LIMIT_GB=500 \
RELAY_IMAGE=commy-relay:staging-iroh-relay-1.4.1 RELAY_IPV6_SUBNET=fd99:6513:f442::/64 \
RELAY_ACCESS_URL=http://relay-access:3342/ RELAY_ACCESS_MODE=watch RELAY_ACCESS_ALIAS=commy-relay-staging-access \
RELAY_ACCOUNTS_NETWORK=commy-staging RELAY_TEST_ACCOUNTS_NETWORK=commy-dev \
RELAY_ACCESS_AUTH_URLS=http://commy-staging-auth:3001/internal/relay-access,http://commy-dev-auth:3001/internal/relay-access \
  docker compose -p commy-relay-staging -f docker-compose.yml -f docker-compose.relay.yml up -d --build relay relay-access
```

Start in `watch` mode: every app is still let in, and `access.json` in the
`relay_meter_data` volume counts the day's apps, how many are not signed in,
and how many checks could not reach auth. `docker compose logs relay-access`
shows each app it refuses (or would refuse) by a short id, at most once per 10
minutes per app. Switch `RELAY_ACCESS_MODE` to
`enforce` once that count only shows strangers. Bots run by server owners are
not signed-in devices, so they use the public relays.

The relay image is tagged with the iroh-relay version it builds, so a version
bump in `relay/Dockerfile` gets a fresh build even without `--build`. To check
which relay is live, `https://relay.<domain>:8443/ping` answers 200 on
iroh-relay 1.x, which the web app needs; an older relay answers 404 and refuses
browsers.

#### Relays on their own server

A relay can move to a second server so its bandwidth never competes with the
sites. The relay names and ports stay the same, so the apps keep working and
only DNS changes. The access checker stays on the main server, because it
reaches auth over the internal Docker networks. The moved relay asks it over
HTTPS on port 8443, through the main edge, which lets only the relay server's
addresses through. The steps below move the test build relay; the release relay
moves the same way with its own names, ports and subnet.

1. Prepare the relay server: SSH key login for a sudo user, then
   `sudo sh deploy/host/harden-vps.sh` from a checkout in `/srv/commy` (it opens
   80/tcp, 8443/tcp and both address discovery ports), Docker Engine with the
   compose plugin, and a swapfile if it has little memory.
2. On the main server, give each access checker a name the edge can reach.
   `docker-compose.relay.yml` puts it on `commy-relay` as
   `RELAY_ACCESS_ALIAS`: `commy-relay-access` for the release relay (the
   default) and `commy-relay-staging-access` for the test build relay, so run
   their commands above again if they predate that. The tracked
   `edge/sites.d/relay-access.caddy` serves `relay-access.<domain>:8443`,
   sends `/prod/` checks to the release checker and every other path to the
   test build one, and lets in only the addresses in `RELAY_HOST_IPS`. Until
   that is set it lets in nobody but the main server itself. Put the relay
   server's addresses in `edge/.env`:

   ```bash
   RELAY_HOST_IPS="<relay server IPv4> <relay server IPv6>"
   ```

   and recreate the edge with `docker compose up -d caddy` in `edge/`, since
   Caddy reads environment values only when it starts.
3. On the relay server, put `ACME_EMAIL`, `RELAY_DOMAIN` and
   `STAGING_RELAY_DOMAIN` in `deploy/relay-host/.env` and run
   `docker compose up -d` there. This small edge creates the `commy-relay`
   network and the `commy-edge_caddy_data` volume that
   `docker-compose.relay.yml` expects, so the relay commands work unchanged.
4. Give `deploy/.env` placeholder values for `DOMAIN`, `ACME_EMAIL`,
   `POSTGRES_PASSWORD` and `BETTER_AUTH_SECRET`: compose requires them, but the
   relay services never read them. Then start the relay without its access
   checker, which stays on the main server:

   ```bash
   cd /srv/commy/deploy
   RELAY_PROXY_ALIAS=commy-relay-staging-proxy RELAY_DOMAIN=relay-staging.commy.chat RELAY_QAD_PORT=7843 RELAY_MONTHLY_LIMIT_GB=500 \
   RELAY_IMAGE=commy-relay:staging-iroh-relay-1.4.1 RELAY_IPV6_SUBNET=fd99:6513:f442::/64 \
   RELAY_ACCESS_URL=https://relay-access.commy.chat:8443/ \
     docker compose -p commy-relay-staging -f docker-compose.yml -f docker-compose.relay.yml up -d --no-deps relay relay-proxy relay-meter
   ```

   The release relay runs next to it in the `commy-relay` project and asks the
   release checker under `/prod/`:

   ```bash
   cd /srv/commy/deploy
   RELAY_DOMAIN=relay.commy.chat RELAY_QAD_PORT=7842 RELAY_MONTHLY_LIMIT_GB=6000 \
   RELAY_IMAGE=commy-relay:iroh-relay-1.4.1 RELAY_IPV6_SUBNET=fd99:6513:f442:1::/64 \
   RELAY_ACCESS_URL=https://relay-access.commy.chat:8443/prod/ \
     docker compose -p commy-relay -f docker-compose.yml -f docker-compose.relay.yml up -d --no-deps relay relay-proxy relay-meter
   ```

   Keep each project's own values in its own env file (`--env-file .env
   --env-file relay-prod.env`) rather than in `deploy/.env`, so one relay's
   settings never reach the other.

   Building the relay image compiles iroh-relay, which needs more memory than a
   small server has. Build it on the main server and copy it over with
   `docker save` and `docker load`, together with the relay tools image.
5. The relay starts without address discovery until its name points at the new
   server. Point the relay name's A and AAAA records at the relay server and
   `relay-access.commy.chat` at the main server, all DNS only. Once the edge
   has the certificate, the relay restarts with address discovery on its own.
   Check that `https://relay-staging.commy.chat:8443/ping` answers 200, that
   the address discovery port answers over UDP (a provider firewall in front of
   the server can block it), and that `docker compose logs relay-access` on the
   main server shows the checks. Then stop `relay`, `relay-proxy` and
   `relay-meter` of that project on the main server and keep `relay-access`.

   To move a relay that is already live without a certificate gap, copy its
   certificate folder (`caddy/certificates/<issuer>/<relay name>/` in the
   `commy-edge_caddy_data` volume) from the main server to the same path on
   the relay server and restart the relay server's Caddy before DNS changes.
   It serves that certificate at once and renews it itself later.

### Dockhand

Dockhand shows the containers the script starts (projects `commy-edge`,
`commy-prod`, `commy-staging` and `commy-dev`), so you can read logs and restart
them. Keep their settings in the `.env` files rather than Dockhand's environment
editor, because the agent reads `.env` when it recreates containers.

### Moving from the shared `commy-public` network

Servers set up before the per-environment networks existed have every
environment on one `commy-public` network. Move them once, environment by
environment, without taking the site down. Name the services on every
`docker compose up` line: a bare `up -d` would also recreate Postgres and Redis.

```bash
for env in prod staging dev; do (cd /srv/commy-$env && git pull); done
for net in commy-prod commy-staging commy-dev commy-relay; do docker network create $net; done
for net in commy-prod commy-staging commy-dev commy-relay; do docker network connect $net commy-edge-caddy-1; done
for env in dev staging prod; do (cd /srv/commy-$env/deploy && docker compose up -d --build agent && docker compose up -d auth web); done
cd /srv/commy-prod/deploy && docker compose -p commy-relay -f docker-compose.yml -f docker-compose.relay.yml up -d relay-proxy
cd /srv/commy-staging/deploy && RELAY_PROXY_ALIAS=commy-relay-staging-proxy RELAY_DOMAIN=relay-staging.commy.chat RELAY_QAD_PORT=7843 RELAY_MONTHLY_LIMIT_GB=500 RELAY_IMAGE=commy-relay:staging-iroh-relay-1.4.1 RELAY_IPV6_SUBNET=fd99:6513:f442::/64 docker compose -p commy-relay-staging -f docker-compose.yml -f docker-compose.relay.yml up -d relay-proxy
cd /srv/commy-prod/deploy/edge && docker compose up -d
docker network inspect commy-public --format '{{range .Containers}}{{.Name}} {{end}}'
docker network rm commy-public
```

The running edge Caddy is attached to the new networks by hand first, so it
keeps reaching each environment while that environment's `auth` and `web` are
recreated on their new network (the recreate restarts them, which the edge
covers with its 15 second retry). The last `up -d` in `edge/` recreates Caddy
on the four networks only. The inspect line should print nothing before the old
network is removed; if it names a container, recreate that one with
`docker compose up -d <service>` in its folder first.

## Cloudflare in front

With the domain's DNS on Cloudflare:

- **DNS**: `A` records for `@`, `staging`, `dev` and `docs` pointing at the server
  IP, all **Proxied**.
- **SSL/TLS > Overview**: **Full (strict)**. Caddy holds a real certificate for
  each name, so Cloudflare can verify the origin.
- **SSL/TLS > Edge Certificates > Always Use HTTPS**: **Off**. Caddy redirects to
  HTTPS itself, and with this on, Let's Encrypt cannot reach Caddy to issue the
  first certificates.
- **Staging and dev behind Cloudflare Access**: Zero Trust > Access >
  Applications > Add > Self-hosted, with `staging.commy.chat` and
  `dev.commy.chat` as the application domains and an Allow policy for your email
  addresses. Testing the desktop app against staging also needs a second
  application with a Bypass policy (include: Everyone) covering
  `staging.commy.chat/api/auth`, `staging.commy.chat/.well-known/commy-auth.json`
  and `staging.commy.chat/sync`,
  since the app cannot complete the Access login and discovers the account server
  through that document first. The approval page the app opens in the browser stays
  behind Access, which is fine because a browser can complete the login.
- **Certificates for hosts behind Access.** Access redirects every plain HTTP
  request, so Let's Encrypt cannot validate staging and dev. Give Caddy a
  Cloudflare origin certificate for them instead: SSL/TLS > Origin Server >
  Create Certificate for `commy.chat` and `*.commy.chat`, then on the server:

  ```bash
  cd /srv/commy-prod/deploy/edge && mkdir -p certs
  nano certs/origin.pem    # paste the certificate
  nano certs/origin.key    # paste the private key
  chmod 600 certs/origin.key
  echo 'tls /etc/caddy/certs/origin.pem /etc/caddy/certs/origin.key' > certs/origin-tls.caddy
  docker compose up -d --force-recreate caddy
  ```

  Only Cloudflare trusts this certificate, which is fine because only Cloudflare
  connects to the server. Production keeps its Let's Encrypt certificate.
- **Lock the origin to Cloudflare** so nobody can skip Access by connecting to the
  server IP directly: allow ports 80 and 443 only from
  [Cloudflare's IP ranges](https://www.cloudflare.com/ips/) in the hosting
  provider's firewall (Hostinger: hPanel > VPS > Security > Firewall).
- **Require this zone's client certificate** on staging and dev. Cloudflare's IP
  ranges are shared by every Cloudflare customer, so someone could point their
  own Cloudflare site at this server's IP and reach staging without Access.
  Per-hostname Authenticated Origin Pulls makes Cloudflare present a certificate
  only this zone holds:

  ```bash
  cd /srv/commy-prod/deploy/edge/certs
  openssl req -x509 -new -nodes -newkey rsa:4096 -keyout pull-ca.key -out pull-ca.pem -days 1826 -subj "/CN=commy.chat origin pulls"
  openssl req -new -nodes -newkey rsa:4096 -keyout pull.key -out pull.csr -subj "/CN=staging.commy.chat"
  printf "basicConstraints=CA:FALSE
" > pull.ext
  openssl x509 -req -in pull.csr -CA pull-ca.pem -CAkey pull-ca.key -CAcreateserial -out pull.pem -days 730 -sha256 -extfile pull.ext
  chmod 600 pull-ca.key pull.key
  ```

  In Cloudflare, SSL/TLS > Origin Server > Authenticated Origin Pulls >
  Per-hostname > Upload certificate, paste `pull.pem` and `pull.key`, and
  associate `staging.commy.chat` and `dev.commy.chat`. Then make Caddy require
  it and restart Caddy:

  ```bash
  cat > origin-tls.caddy <<'EOF'
  tls /etc/caddy/certs/origin.pem /etc/caddy/certs/origin.key {
  	client_auth {
  		mode require_and_verify
  		trust_pool file /etc/caddy/certs/pull-ca.pem
  	}
  }
  EOF
  cd .. && docker compose up -d --force-recreate caddy
  ```

  A request straight to the server IP for staging now fails the TLS handshake.
  Keep `pull-ca.key` off the server once the certificate is made.

The edge Caddy trusts `CF-Connecting-IP` from Cloudflare's ranges, so logs and
the auth server's rate limits see the visitor's real address.

## Turning on the sync service

New environments from `setup-vps.sh` run it on staging and dev already. On an
environment set up before it existed, add to that environment's `deploy/.env`
(dev shown; staging uses `commy-sync:staging` and `staging.commy.chat`):

```bash
SYNC_IMAGE=git.commy.chat/commy/commy-sync:dev
AGENT_SERVICES=auth web sync
SYNC_URL=wss://dev.commy.chat/sync
```

Then pull the checkout and start it, one service at a time:

```bash
cd /srv/commy-dev && git pull
cd deploy
docker compose up -d --no-deps agent
docker compose up -d --no-deps sync
docker compose up -d --no-deps auth
cd /srv/commy-prod/deploy/edge && docker compose up -d --force-recreate caddy
```

`https://dev.commy.chat/sync/healthz` then answers `{"ok":true,...}` and the
discovery document lists `endpoints.sync`. Behind Cloudflare Access, add
`dev.commy.chat/sync` (and the staging one) to the Bypass application, since
the apps cannot complete the Access login. Production stays off until its
`.env` gets the same three lines.

### Relays for hosted communities

Apps signed in here use the shared relay in every community. In communities
synced here, members whose account is on another server use the free public
relays for voice instead, so other homes never add to the relay bill. The per
environment stacks announce no relay (`SYNC_RELAY_URL` stays empty) and tell
those apps to use the public relays (`SYNC_RELAY_FALLBACK=public` by default).
`GET /sync/about` shows `"relay_fallback":"public"`. The relay access check
can stay on `enforce`.

### The push gateway

Self-hosted sync servers cannot wake phones on their own, because the app's
push keys stay with us. The gateway lets them ask our sync service to send a
wake-up for them, and keeps the cost bounded:

- Each request is signed with the server's published server key, so only the
  server at that domain can send as it.
- Only sealed previews to real phone push tokens are carried. Plain text is
  refused.
- Every server has a daily cap that grows with a clean record: 500 pings and 25
  new phones a day for its first week, 5,000 and 250 after 7 clean days, and
  50,000 and 2,500 after 30.
- A server that keeps sending ten times past its cap is cut off on its own, and
  stays cut off until an operator restores it.
- Servers on the signed server blocklist are refused. It is read from
  `SYNC_GATEWAY_BLOCKLIST` (commy.chat when empty) every 15 minutes, and the
  last good list is kept if a read fails.
- `SYNC_GATEWAY_DAILY_BUDGET` caps all servers together. At
  `SYNC_GATEWAY_ALERT_PERCENT` and again when it runs out, an alert goes to
  `ALERT_WEBHOOK_URL` and, with SMTP set, to `SYNC_ABUSE_CONTACT`.

It is off until that environment's `deploy/.env` has `SYNC_PUSH_GATEWAY=on`
(then `docker compose up -d --no-deps sync`). To see and change who uses it:

```bash
docker compose exec sync commy-sync gateway
docker compose exec sync commy-sync gateway cut https://bad.example.org sent spam
docker compose exec sync commy-sync gateway restore https://bad.example.org
docker compose exec sync commy-sync gateway tier https://good.example.org trusted
docker compose exec sync commy-sync gateway tier https://good.example.org auto
```

With `SYNC_ADMIN_PASSWORD` set, the admin page shows the same list with today's
use per server and buttons for the same changes.

## Turning on the server listing

The list of community servers ([LISTING.md](LISTING.md)) runs on the official
account servers only. Staging first; production only with the owner's go. Add
to that environment's `deploy/.env` (staging shown):

```bash
LISTING=on
LISTING_REVIEW_EMAIL=<the listing review inbox>
COMMY_ABUSE_CONTACT=<the abuse contact>
COMMY_TERMS_URL=https://staging.commy.chat/terms
COMMY_PRIVACY_URL=https://staging.commy.chat/privacy
```

Then restart auth only:

```bash
cd /srv/commy-staging/deploy && docker compose up -d --no-deps auth
```

The discovery document then lists `serverList`, and
`/api/auth/commy/server-list` answers a signed, empty list. The form lives at
`/servers/apply` and the review page at `/servers/review`; on commy.chat the
website forwards both paths to the auth server's render endpoint, like the
account pages. `docker compose exec auth node src/listing-cli.js list` shows
every application.

## Hardening the server

After `setup-vps.sh`, run once:

```bash
sudo sh /srv/commy-prod/deploy/host/harden-vps.sh
```

- **ufw** denies incoming connections except SSH (rate limited), port 80 and
  the iroh relay ports (`PUBLIC_PORTS`, default `7842/udp 7843/udp 8443/tcp`),
  which stay open to everyone because clients reach the relay directly.
- **SSH with keys only.** A drop-in in `/etc/ssh/sshd_config.d/` turns off
  password and keyboard-interactive login and closes root login (key-only root
  login when the script is run as root rather than through a sudo user). The
  script first checks that the user running it, or root, has a key in
  `~/.ssh/authorized_keys`; without one it leaves password login on and prints
  how to add a key, so it can never lock you out. The new settings are checked
  with `sshd -t` before sshd is reloaded. The hosting provider's browser
  console reaches sshd from the link-local network `169.254.0.0/16`, so a
  `Match Address` block at the end of `sshd_config` keeps password login open
  for that network only, and fail2ban never bans it. Set `CONSOLE_NET` to
  another network, or to nothing to close the console too.
- **fail2ban** with an sshd jail (5 failed attempts in 10 minutes ban the
  address for an hour) and **unattended-upgrades** with the standard daily
  security updates.
- **HTTPS only from Cloudflare.** Port 443 is accepted only from
  [Cloudflare's ranges](https://www.cloudflare.com/ips/), both in ufw and in
  Docker's `DOCKER-USER` chain, since ports published by Docker skip ufw. This
  is what stops anyone from bypassing Cloudflare Access on staging by connecting
  to the server IP. `commy-firewall.timer` refreshes the ranges daily, and a
  Docker drop-in re-applies them whenever Docker restarts. Port 80 stays open
  because Caddy only redirects there and Let's Encrypt needs it.
- **Containers** run with `no-new-privileges`, every capability dropped (Caddy
  keeps `NET_BIND_SERVICE` for ports 80 and 443) and pid and memory limits;
  `WEB_MEM_LIMIT`, `AUTH_MEM_LIMIT`, `RELAY_MEM_LIMIT` and `CADDY_MEM_LIMIT` in
  `.env` raise the memory limits. The edge publishes 443 over TCP only, since
  Cloudflare reaches the origin over TCP.
- **Access logs.** The edge Caddy writes JSON access logs with real visitor IPs
  (from `CF-Connecting-IP`) to `/var/log/commy-caddy`, so any log or intrusion
  detection tool you already run can read them. Commy does not install one.

To let your own IP reach port 443 directly (for example to test without
Cloudflare), put `COMMY_FW_ALLOW="203.0.113.7/32"` in `/etc/default/commy-firewall`
and run `sudo commy-firewall apply`. The rule applies to the interface of the
default route; set `COMMY_FW_IFACE` in the same file if the public interface is
a different one. `sudo commy-firewall remove` undoes the Cloudflare-only rule.

## One environment only

For a single server running only production with the stack's own Caddy, follow
[HOSTING.md](HOSTING.md), then add these lines to `deploy/.env` and run
`docker compose up -d`:

```bash
COMPOSE_FILE=docker-compose.yml:docker-compose.agent.yml
WEB_IMAGE=git.commy.chat/commy-private/commy-web:prod
AUTH_IMAGE=git.commy.chat/commy/commy-auth:prod
AGENT_VERIFY=on
```

## Shipping a change

- **Dev**: merge a pull request. The dev site updates within about 30 seconds after the
  **Server images** workflow finishes (for the website: the **Website image**
  workflow in the commy-web repository).
- **Staging**: run the "Cut release" workflow in both repositories with the same version (or push a release tag such as `v0.2.0` to each). Suffixed versions like `0.2.0-rc.1` are published as pre-releases, and their desktop builds sign in to staging.commy.chat by default.
  The same tag also builds the desktop release. A tag containing `-test` (such
  as `v0.2.0-test.1`) publishes that desktop release as a draft.
- **Prod**: the owner signs the exact images staging is running with the prod
  key in Azure Key Vault and moves the `:prod` tags, from a machine signed in
  to Azure with `az login` (see below). The forge workflows only promote to
  staging.

### Signed promotes

Prod accepts only images signed with the prod key, an EC P-256 key that lives in
Azure Key Vault and can be used but never downloaded. Its public key is
`deploy/agent/cosign-prod.pub`, and the prod environment's `.env` sets
`AGENT_COSIGN_KEY=/repo/deploy/agent/cosign-prod.pub`. To promote, for each of
`commy/commy-auth`, `commy/commy-sync` and `commy-private/commy-web`:

```bash
digest="$(docker buildx imagetools inspect "$repo:staging" --format '{{json .Manifest.Digest}}' | tr -d '"')"
cosign sign --yes --key azurekms://<vault>.vault.azure.net/<key> -a tag=prod "$repo@$digest"
docker buildx imagetools create --tag "$repo:prod" "$repo@$digest"
cosign verify -a tag=prod --key deploy/agent/cosign-prod.pub "$repo:prod"
```

Use a release version instead of `staging` to roll back.

Staging promotes run on the forge as below.

**Promote images** signs the promoted digest with
[cosign](https://github.com/sigstore/cosign) using the Commy signing key before it
moves the tag, and the signature names the tag it was promoted to. The private key
and its password are the forge secrets `COSIGN_PRIVATE_KEY` and `COSIGN_PASSWORD`;
the public key is `deploy/agent/cosign.pub`. Only the accounts in the `PROMOTERS`
variable (by default `commy-commander`) can run the workflow, and only from `main`.
Forge secrets reach every workflow on every branch, so anyone who can push to
the repository can reach the staging signing key; keep write access to the owner.
Each agent runs
`cosign verify` against its environment's public key on every new image before
starting it, so an image retagged by anything else, or a staging signature moved
onto the prod tag, is refused and logged; the running container keeps serving and the
image is tried again on the next cycle, in case the signature was still being
written. Settings in that environment's `.env`:

- `AGENT_VERIFY`: `auto` (default) checks signatures for any image tagged
  `:prod` and for every image when `COMPOSE_PROJECT_NAME` contains `prod`, and
  refuses those images when cosign is missing; `on` checks everywhere; `off`
  never checks.
- `AGENT_COSIGN_KEY`: the public key to accept, by default
  `/repo/deploy/agent/cosign.pub` (the checkout the agent mounts). A fork signs
  with its own key pair and points this at its public key. Set it empty to fall
  back to keyless signatures, checked with `AGENT_COSIGN_IDENTITY`,
  `AGENT_COSIGN_WEB_IDENTITY` and `AGENT_COSIGN_ISSUER`.

To check an image by hand from any machine with cosign:

```bash
cosign verify -a tag=prod --key deploy/agent/cosign.pub git.commy.chat/commy/commy-auth:prod
```

Changes to the compose files themselves (a new environment variable, a new
service) are not picked up by the agent: run `git pull` and then
`docker compose up -d <service>` for the changed services in that environment's
`deploy` folder (`docker compose up -d --build agent` after a change to the
agent's image).

## Rolling back

Run **Promote images** with the environment and the version to go back to in
**source**, for example `0.1.0`. The agent deploys it like any other update.
To pause automatic updates, `docker compose stop agent`.

## One-time forge setup

- **Package visibility.** The auth and sync images live in the forge registry
  under the `commy` organisation, which is public, so they pull without a login.
  The website image lives under the private `commy-private` organisation. Run
  `sudo docker login git.commy.chat` on the server once with a token that has
  only `read:package`; the agent reads that login from `/root/.docker`
  (`AGENT_DOCKER_CONFIG` points it elsewhere), and CI jobs that pull the website
  log in with `PACKAGES_TOKEN`.
- **Image pushes.** CI pushes with the `PACKAGES_TOKEN` secret, a token with
  `write:package`, falling back to the job token.
- **Release hosting.** Releases publish to the Cloudflare R2 bucket `commy-releases`,
  served at releases.commy.chat. `node deploy/releases/publish.mjs <tag> <dist-dir>`
  uploads a built release (add `--dry-run` to see the plan): files go under the tag,
  then `stable/` or `rc/` gets `release.json` and, last, `latest.json`. Tags with
  `-test` never move a channel. `--files <folder> <dir>` uploads other immutable
  files, such as the voice pack, under `<folder>/`. It needs `R2_ACCOUNT_ID`,
  `R2_ACCESS_KEY_ID` and `R2_SECRET_ACCESS_KEY` (an R2 token with Object Read and
  Write on that bucket only) as repository secrets.
- **Preview nightlies.** With the repository variable `NIGHTLY_BUILDS` set to `true`,
  "Desktop release" runs every night at 04:17 UTC from `main` and, when `main` changed
  since the last one, builds a Preview nightly such as `1.0.1-nightly.20261118.42` (the
  version after the newest stable). `publish.mjs --nightly <version> <dist-dir>` puts
  its files under `nightly/<version>/`, moves `nightly/latest.json` last, and keeps the
  seven newest nightlies. Nightlies use the production servers, skip the forge and
  GitHub releases and the download page, and reach only desktop apps whose Update
  channel is Preview. "Run workflow" with "nightly" ticked builds one by hand. With the
  variable unset, the nightly schedule only rebuilds on Mondays and Thursdays to keep
  the build caches warm.
- **GitHub copy.** GitHub keeps a light copy of each final release: after a release
  publishes, `deploy/releases/github-copy.mjs` pushes `main` and the new tag to
  github.com/commy-commander/commy and creates the GitHub release with the forge notes,
  `latest.json` and the checksums (installers stay on releases.commy.chat). The
  "Copy releases to GitHub" workflow repeats this when forge notes are edited and twice a
  day, and only ever fast-forwards. Add `--dry-run` to see the plan. It needs the
  `MIRROR_GITHUB_TOKEN` secret, a fine-grained GitHub token for that repository with
  Contents and Workflows set to Read and write. Actions stay disabled on the GitHub copy.
- **Desktop update signing.** Installed desktop apps update themselves from
  `https://releases.commy.chat/<channel>/latest.json` (`rc` for test builds, `stable`
  for the rest, and both `nightly` and `stable` for the Preview channel),
  and install only files signed with the key whose public half is
  `plugins.updater.pubkey` in `apps/desktop/src-tauri/tauri.conf.json`. Make the key
  once with `npx @tauri-apps/cli@2 signer generate -w commy-updater.key`, put the
  private key and its password in the repository secrets `TAURI_SIGNING_PRIVATE_KEY`
  and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, and keep a backup: copies already
  installed can never update again if it is lost. Without the secrets, releases
  publish without `latest.json` and installed apps do not see them.

## Testing these scripts

`sh deploy/agent/commy-agent.test.sh` and `sh deploy/host/commy-firewall.test.sh`
run against fake `docker`, `iptables`, `ufw` and `curl` commands, so they need
nothing installed. `sh deploy/agent/integration.test.sh` runs the agent against
a real Docker engine with Compose v2 and a local registry.
`sh deploy/edge/edge.test.sh` checks the edge Caddyfile in a real Caddy, which it
downloads unless `CADDY` names one.
