#!/bin/sh

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
		if [ "$env" = dev ]; then echo "AGENT_INTERVAL=10" >>"$envfile"; fi
		if [ -n "${AGENT_VERIFY:-}" ]; then echo "AGENT_VERIFY=$AGENT_VERIFY" >>"$envfile"; fi
		chmod 600 "$envfile"
		echo "commy-$env: wrote $envfile (add Google, Twitch and SMTP settings there later)"
	fi
done

first="$(echo "$ENVS" | cut -d' ' -f1)"
for net in commy-prod commy-staging commy-dev commy-relay; do
	docker network inspect "$net" >/dev/null 2>&1 || docker network create "$net" >/dev/null
done
edge="$ROOT/commy-$first/deploy/edge"
if [ ! -f "$edge/.env" ]; then
	printf 'ACME_EMAIL=%s\nPROD_DOMAIN=%s\n' "$EMAIL" "$DOMAIN" >"$edge/.env"
fi
(cd "$edge" && docker compose up -d)
echo "edge: Caddy is serving $DOMAIN, staging.$DOMAIN and dev.$DOMAIN"

for env in $ENVS; do
	cd "$ROOT/commy-$env/deploy"
	docker compose up -d postgres redis agent
	echo "commy-$env: database, redis and deploy agent running"
done

relay_dir="$ROOT/commy-$first/deploy"
if [ -f "$relay_dir/docker-compose.relay.yml" ]; then
	(cd "$relay_dir" && docker compose -p commy-relay -f docker-compose.yml -f docker-compose.relay.yml \
		up -d --build relay relay-proxy relay-meter)
	echo "relay: running for relay.$DOMAIN"
fi
if echo " $ENVS " | grep -q " staging " && [ -f "$ROOT/commy-staging/deploy/docker-compose.relay.yml" ]; then
	(cd "$ROOT/commy-staging/deploy" && RELAY_PROXY_ALIAS=commy-relay-staging-proxy STUN_PORT=3479 RELAY_MONTHLY_LIMIT_GB=500 \
		docker compose -p commy-relay-staging -f docker-compose.yml -f docker-compose.relay.yml \
		up -d --build relay relay-proxy relay-meter)
	echo "relay: running for relay-staging.$DOMAIN (test builds)"
fi

cat <<EOF

Done. Each environment's agent pulls its image tag every minute (dev every 10s):
  dev     <- every merge to main
  staging <- every release tag (v*)
  prod    <- the "Promote images" workflow in GitHub Actions
Watch one with: cd $ROOT/commy-staging/deploy && docker compose logs -f agent
EOF
