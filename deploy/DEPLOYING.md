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
  `commy-relay` project, from the prod checkout, STUN on 3478/udp) and
  `relay-staging.<domain>` for test builds (the `commy-relay-staging` project,
  from the staging checkout, STUN on 3479/udp). Both are DNS-only records, and
  `setup-vps.sh` starts both. Test builds never use the production relay or
  account server, and release builds never use staging.
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
cd /srv/commy-staging/deploy && RELAY_PROXY_ALIAS=commy-relay-staging-proxy STUN_PORT=3479 RELAY_MONTHLY_LIMIT_GB=500 docker compose -p commy-relay-staging -f docker-compose.yml -f docker-compose.relay.yml up -d relay-proxy
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
  `staging.commy.chat/api/auth` and `staging.commy.chat/.well-known/commy-auth.json`,
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

The edge Caddy trusts `CF-Connecting-IP` from Cloudflare's ranges, so logs and
the auth server's rate limits see the visitor's real address.

## Hardening the server

After `setup-vps.sh`, run once:

```bash
sudo sh /srv/commy-prod/deploy/host/harden-vps.sh
```

- **ufw** denies incoming connections except SSH (rate limited), port 80 and
  the iroh relay ports (`PUBLIC_PORTS`, default `3478/udp 3479/udp 8443/tcp`),
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
and run `sudo commy-firewall apply`. `sudo commy-firewall remove` undoes the
Cloudflare-only rule.

## One environment only

For a single server running only production with the stack's own Caddy, follow
[HOSTING.md](HOSTING.md), then add these lines to `deploy/.env` and run
`docker compose up -d`:

```bash
COMPOSE_FILE=docker-compose.yml:docker-compose.agent.yml
WEB_IMAGE=ghcr.io/commy-commander/commy-web:prod
AUTH_IMAGE=ghcr.io/commy-commander/commy-auth:prod
```

## Shipping a change

- **Dev**: merge a pull request. The dev site updates within about 30 seconds after the
  **Server images** workflow finishes (for the website: the **Website image**
  workflow in the commy-web repository).
- **Staging**: run the "Cut release" workflow in both repositories with the same version (or push a release tag such as `v0.2.0` to each). Suffixed versions like `0.2.0-rc.1` are published as pre-releases, and their desktop builds sign in to staging.commy.chat by default.
  The same tag also builds the desktop release.
- **Prod**: Actions > **Promote images** > Run workflow > `prod`. A required
  reviewer approves it under the run's **Review deployments** button, then prod
  picks up the exact images staging is running. The website has the same
  **Promote image** workflow in the commy-web repository; run both. The
  workflow only accepts `staging`, `dev` (for staging) or a release version as
  the source.

### Signed promotes

**Promote images** signs the promoted digest with
[cosign](https://github.com/sigstore/cosign) (keyless, through GitHub's OIDC
token) before it moves the tag, and the signature names the tag it was promoted
to. The prod agent runs `cosign verify` on every new image before starting it
and only accepts a signature made by this repository's `promote.yml` on `main`
for that tag, so an image retagged as `:prod` by anything else, or a staging
signature moved onto the prod tag, is refused and logged; the running
container keeps serving and the image is tried again on the next cycle, in case
the signature was still being written. Settings in that environment's `.env`:

- `AGENT_VERIFY`: `auto` (default) checks signatures when `COMPOSE_PROJECT_NAME`
  contains `prod` and cosign is in the agent image; `on` checks everywhere and
  fails closed; `off` never checks. A single-environment server that runs the
  `:prod` tags under another project name should set `on`.
- `AGENT_COSIGN_IDENTITY`: the workflow identity to accept, by default
  `https://github.com/commy-commander/commy/.github/workflows/promote.yml@refs/heads/main`.
  A fork promoting its own images sets its own repository here.
- `AGENT_COSIGN_ISSUER`: the OIDC issuer, by default GitHub Actions.

To check an image by hand from any machine with cosign:

```bash
cosign verify -a tag=prod \
  --certificate-identity https://github.com/commy-commander/commy/.github/workflows/promote.yml@refs/heads/main \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  ghcr.io/commy-commander/commy-auth:prod
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

## One-time GitHub setup

- **Package visibility.** The first image push creates `commy-web` and
  `commy-auth` under the organisation's Packages tab as private. Make both public
  (Package settings > Change visibility) so servers can pull without a login, or
  run `docker login ghcr.io` on the server with a token that has `read:packages`
  and mount `/root/.docker/config.json` into the agent.
- **Environments.** Settings > Environments needs `production` with a required
  reviewer, which gates promotion to prod, and `staging` without one.
- **Desktop update signing.** Installed desktop apps update themselves from the
  releases on their channel (pre-releases for test builds, finals for the rest),
  and install only files signed with the key whose public half is
  `plugins.updater.pubkey` in `apps/desktop/src-tauri/tauri.conf.json`. Make the key
  once with `npx @tauri-apps/cli@2 signer generate -w commy-updater.key`, put the
  private key and its password in the repository secrets `TAURI_SIGNING_PRIVATE_KEY`
  and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, and keep a backup: copies already
  installed can never update again if it is lost. Without the secrets, releases
  publish without `latest.json` and installed apps do not see them.
