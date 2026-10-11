---
title: Host your own
description: Run the Commy website, accounts and invite links on your own VPS with Docker Compose.
order: 6
---

Chat, voice and files are already self hosted by design: they live on members' machines. The hosted pieces, the website, accounts and invite links, run from one Docker Compose stack you can put on any small VPS.

## What the stack runs

- **caddy:** the public entry point on ports 80 and 443, with automatic HTTPS.
- **web:** this website and docs.
- **auth:** a Better Auth server for accounts, desktop sign in, device linking and invite links.
- **postgres:** the account database, never exposed publicly.

## Install

Point your domain at the server, then with Docker installed:

```bash
git clone https://github.com/commy-commander/commy
cd commy/deploy
cp .env.example .env
# set DOMAIN, ACME_EMAIL, POSTGRES_PASSWORD and BETTER_AUTH_SECRET
docker compose up -d --build
```

Tables are created on first boot. Google sign in, email (magic links, verification, password reset) and the Turnstile bot check switch on when you add their keys to `.env`. The full guide is [deploy/HOSTING.md](https://github.com/commy-commander/commy/blob/main/deploy/HOSTING.md).

## Pointing Commy at your server

Launch the app with `COMMY_AUTH_URL=https://your-domain` to use your server for accounts and invite links. Invite links your users make will then be `https://your-domain/i/...`.

## Running your own iroh relay

For independence from public relays you can run `iroh-relay` next to the stack. It needs its own hostname and UDP port 3478, and must match the iroh version of the app.
