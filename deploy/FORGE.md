# Code hosting and CI

The code lives on a self-hosted Forgejo at https://git.commy.chat (`commy/commy`
public, `commy/commy-web` private). Release files do not live on the forge: they are
on releases.commy.chat (see DEPLOYING.md, "Release hosting"), so a forge outage never
stops downloads or updates.

## Where things run

| Piece | Host | Files |
|-------|------|-------|
| Forgejo (git, pull requests, Actions queue, package registry) | VPS, `/srv/commy-forge` | `deploy/forge/` |
| Route for git.commy.chat | VPS edge Caddy | `deploy/edge/` |
| Linux jobs (tests, Linux builds, Android, images) | Docker Desktop on the build PC | `deploy/forge-runner/` |
| Windows desktop builds | build PC, host runner (not set up yet) | |
| macOS builds | a Mac, host runner, only when cutting a release | |

The VPS only serves git. It never runs builds.

## The forge

```bash
cd /srv/commy-forge && docker compose up -d
```

Registration is closed. The admin is `commy-commander`; its first password is in
`/srv/commy-forge/admin-password` (owner only). The edge joins the `commy-forge`
network, so start the forge before recreating the edge. Workflows in
`.github/workflows` run as Forgejo Actions; `uses:` actions are fetched from github.com.

Settings applied to both repositories: squash merge only, branches deleted on merge,
`main` protected (pull requests only, admins included).

## A runner on the build PC

Copy `deploy/forge-runner/` to a folder outside the repository, then register it once
with a shared secret, which never leaves the two machines:

```bash
node -e "process.stdout.write(require('crypto').randomBytes(20).toString('hex'))" > runner-secret
ssh commy-vps "docker exec -i commy-forge forgejo forgejo-cli actions register --name commy-pc --scope commy --secret-file /dev/stdin" < runner-secret
docker compose up -d --wait dind
docker compose run --rm --no-deps -T -v "$PWD/runner-secret:/run/runner-secret:ro" --entrypoint sh runner \
  -c 'forgejo-runner create-runner-file --connect --instance https://git.commy.chat --name commy-pc --secret "$(cat /run/runner-secret)"'
rm runner-secret
docker compose up -d
```

`CI_CPUS` and `CI_MEMORY` in its `.env` cap each container. The labels in
`config.yaml` map `ubuntu-latest`, `ubuntu-24.04`, `ubuntu-22.04`, `commy-ci` and
`commy-pc` to the `catthehacker/ubuntu:full-*` images (about 19 GB each, pulled once),
and jobs reach the runner's own Docker daemon through `DOCKER_HOST`.

## Secrets

Repository secrets (Settings > Actions > Secrets): `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`,
`R2_SECRET_ACCESS_KEY` for publishing releases, and `TAURI_SIGNING_PRIVATE_KEY` with
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` for signing updates.

## Mirroring to GitHub

When the GitHub account is available, add a push mirror on each repository
(Settings > Repository > Mirror settings): remote
`https://github.com/commy-commander/<repo>.git`, user `commy-commander`, a GitHub
fine-grained token with Contents read and write on that repository, and "sync when
commits are pushed". The forge stays the source of truth; GitHub only receives.

## Container images

Server and website images live in the forge registry, `git.commy.chat/commy/<image>`.
CI pushes with the `PACKAGES_TOKEN` secret (a token with `write:package`), and
**Promote images** signs with the key pair described in DEPLOYING.md. Uploads go
through the Cloudflare proxy, which caps a single request at 100 MB, so a layer
larger than that needs git.commy.chat set to DNS only.
