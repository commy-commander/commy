#!/bin/sh
# One-time setup of dev, staging and prod on a fresh Ubuntu server with Docker.
#   curl -fsSL https://raw.githubusercontent.com/commy-commander/commy/main/deploy/setup-vps.sh | sudo sh -s -- commy.chat you@example.org
# Safe to run again: existing checkouts and .env files are kept.

set -eu

DOMAIN="${1:?usage: setup-vps.sh <domain> <acme-email> [envs]}"
EMAIL="${2:?usage: setup-vps.sh <domain> <acme-email> [envs]}"
ENVS="${3:-prod staging dev}"
REPO="${COMMY_REPO:-https://github.com/commy-commander/commy.git}"
ROOT="${COMMY_ROOT:-/srv}"
IMAGES="${COMMY_IMAGES:-ghcr.io/commy-commander}"

command -v docker >/dev/null || { echo "Docker is not installed"; exit 1; }
docker compose version >/dev/null || { echo "Docker Compose v2 is not installed"; exit 1; }
command -v git >/dev/null || { apt-get update -qq && apt-get install -y -qq git; }

# Ports 80 and 443 must be free for the edge Caddy (or already be ours)
for port in 80 443; do
	owner="$(docker ps --filter "publish=$port" --format '{{.Names}}' | grep -v '^commy-edge-' || true)"
	if [ -n "$owner" ]; then
		echo "Port $port is used by container(s): $owner"
		echo "Stop or move them first (for example a Dockhand-managed proxy), then run this again."
		exit 1
	fi
	if command -v ss >/dev/null && [ -z "$(docker ps -q --filter "publish=$port")" ] \
		&& ss -ltnH "sport = :$port" | grep -q .; then
		echo "Port $port is used by a service outside Docker:"
		ss -ltnpH "sport = :$port"
		echo "Stop it first (for example: systemctl disable --now apache2 nginx), then run this again."
		exit 1
	fi
done

host_for() {
	if [ "$1" = prod ]; then echo "$DOMAIN"; else echo "$1.$DOMAIN"; fi
}

for env in $ENVS; do
	dir="$ROOT/commy-$env"
	[ -d "$dir/.git" ] || git clone -q "$REPO" "$dir"
	envfile="$dir/deploy/.env"
	if [ ! -f "$envfile" ]; then
		cat >"$envfile" <<EOF
COMPOSE_PROJECT_NAME=commy-$env
COMPOSE_FILE=docker-compose.yml:docker-compose.agent.yml:docker-compose.env.yml
DOMAIN=$(host_for "$env")
ACME_EMAIL=$EMAIL
POSTGRES_PASSWORD=$(openssl rand -hex 24)
BETTER_AUTH_SECRET=$(openssl rand -base64 32)
WEB_IMAGE=$IMAGES/commy-web:$env
AUTH_IMAGE=$IMAGES/commy-auth:$env
EOF
		chmod 600 "$envfile"
		echo "commy-$env: wrote $envfile (add Google and SMTP settings there later)"
	fi
done

first="$(echo "$ENVS" | cut -d' ' -f1)"
edge="$ROOT/commy-$first/deploy/edge"
if [ ! -f "$edge/.env" ]; then
	printf 'ACME_EMAIL=%s\nPROD_DOMAIN=%s\n' "$EMAIL" "$DOMAIN" >"$edge/.env"
fi
(cd "$edge" && docker compose up -d)
echo "edge: Caddy is serving $DOMAIN, staging.$DOMAIN and dev.$DOMAIN"

for env in $ENVS; do
	cd "$ROOT/commy-$env/deploy"
	# The agent starts auth and web as soon as their image tag exists on GHCR
	docker compose up -d postgres agent
	echo "commy-$env: database and deploy agent running"
done

cat <<EOF

Done. Each environment's agent pulls its image tag every minute:
  dev     <- every merge to main
  staging <- every release tag (v*)
  prod    <- the "Promote images" workflow in GitHub Actions
Watch one with: cd $ROOT/commy-staging/deploy && docker compose logs -f agent
EOF
