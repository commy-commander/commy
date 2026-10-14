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

Every build is also kept as `:sha-<commit>` and every release as `:<version>`,
which is what makes rollback possible.

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
   `postgres` and `caddy` are never touched.

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
  Port 8443 serves the shared iroh relay at `relay.<domain>` (a DNS-only record),
  which `setup-vps.sh` starts once as the `commy-relay` project.
- `docker-compose.env.yml` turns off the per-stack Caddy and joins `web` and
  `auth` to the shared `commy-public` network under those aliases. Each
  environment's Postgres stays on its own private network.
- `docker-compose.agent.yml` adds the agent. It mounts the checkout read only and
  the Docker socket.

### Dockhand

Dockhand shows the containers the script starts (projects `commy-edge`,
`commy-prod`, `commy-staging` and `commy-dev`), so you can read logs and restart
them. Keep their settings in the `.env` files rather than Dockhand's environment
editor, because the agent reads `.env` when it recreates containers.

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
  the iroh relay ports (`PUBLIC_PORTS`, default `3478/udp 7842/udp 8443/tcp`),
  which stay open to everyone because clients reach the relay directly.
- **HTTPS only from Cloudflare.** Port 443 is accepted only from
  [Cloudflare's ranges](https://www.cloudflare.com/ips/), both in ufw and in
  Docker's `DOCKER-USER` chain, since ports published by Docker skip ufw. This
  is what stops anyone from bypassing Cloudflare Access on staging by connecting
  to the server IP. `commy-firewall.timer` refreshes the ranges daily, and a
  Docker drop-in re-applies them whenever Docker restarts. Port 80 stays open
  because Caddy only redirects there and Let's Encrypt needs it.
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
- **Staging**: push a release tag (`git tag v0.2.0 && git push origin v0.2.0`).
  The same tag also builds the desktop release.
- **Prod**: Actions > **Promote images** > Run workflow > `prod`. A required
  reviewer approves it under the run's **Review deployments** button, then prod
  picks up the exact images staging is running. The website has the same
  **Promote image** workflow in the commy-web repository; run both.

Changes to the compose files themselves (a new environment variable, a new
service) are not picked up by the agent: run
`git pull && docker compose up -d` in that environment's `deploy` folder.

## Rolling back

Run **Promote images** with the environment and the version to go back to in
**source**, for example `0.1.0` or `sha-1a2b3c4`. The agent deploys it like any
other update. To pause automatic updates, `docker compose stop agent`.

## One-time GitHub setup

- **Package visibility.** The first image push creates `commy-web` and
  `commy-auth` under the organisation's Packages tab as private. Make both public
  (Package settings > Change visibility) so servers can pull without a login, or
  run `docker login ghcr.io` on the server with a token that has `read:packages`
  and mount `/root/.docker/config.json` into the agent.
- **Environments.** Settings > Environments needs `production` with a required
  reviewer, which gates promotion to prod, and `staging` without one.
