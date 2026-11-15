#!/bin/sh

set -eu

FORGE="${COMMY_FORGE:-https://git.commy.chat/commy/commy}"
RELEASES="${COMMY_RELEASES:-https://releases.commy.chat/stable/latest.json}"
IMAGES="${COMMY_IMAGES:-git.commy.chat/commy}"
COSIGN_IMAGE="${COMMY_COSIGN_IMAGE:-ghcr.io/sigstore/cosign/cosign:v2.6.1}"
VERIFY="${COMMY_VERIFY:-on}"
BUNDLE="${COMMY_BUNDLE:-}"
DIR="${COMMY_DIR:-/srv/commy-sync}"
HEALTH_TIMEOUT="${COMMY_HEALTH_TIMEOUT:-300}"
relay_port="${RELAY_QAD_PORT:-7842}"
YES="${COMMY_YES:-}"
domain="${SYNC_DOMAIN:-}"
email="${ACME_EMAIL:-}"
contact="${SYNC_ABUSE_CONTACT:-}"
version="${COMMY_VERSION:-}"
admin="${COMMY_ADMIN:-}"
admin_port="${COMMY_ADMIN_PORT:-8790}"

usage() {
	cat <<EOF
Sets up or updates a Commy sync server with Docker.

usage: setup.sh [--domain sync.example.org] [--email you@example.org]
                [--contact abuse@example.org] [--admin | --no-admin]
                [--version 0.2.5] [--dir /srv/commy-sync] [--yes]

Anything left out is asked for, or read from an earlier run's settings.
Run it again at any time to update to the newest Commy release.
--admin turns on a password-protected admin page that opens only through an SSH tunnel.
EOF
}

say() { printf '%s\n' "$*"; }
fail() {
	printf 'setup: %s\n' "$*" >&2
	exit 1
}

while [ $# -gt 0 ]; do
	case "$1" in
	--domain) domain="${2:?--domain needs a value}"; shift 2 ;;
	--email) email="${2:?--email needs a value}"; shift 2 ;;
	--contact) contact="${2:?--contact needs a value}"; shift 2 ;;
	--admin) admin=on; shift ;;
	--no-admin) admin=off; shift ;;
	--version) version="${2:?--version needs a value}"; shift 2 ;;
	--dir) DIR="${2:?--dir needs a value}"; shift 2 ;;
	--yes | -y) YES=1; shift ;;
	--help | -h) usage; exit 0 ;;
	*) usage >&2; exit 2 ;;
	esac
done

interactive() {
	[ -z "$YES" ] && [ -r /dev/tty ] && { : </dev/tty; } 2>/dev/null
}

ask() {
	interactive || fail "$1 is needed: pass $2"
	printf '%s: ' "$1" >/dev/tty
	IFS= read -r answer </dev/tty || answer=""
	printf '%s' "$answer"
}

confirm() {
	interactive || return 0
	printf '%s [y/N] ' "$1" >/dev/tty
	IFS= read -r answer </dev/tty || answer=""
	case "$answer" in y | Y | yes | Yes) return 0 ;; *) return 1 ;; esac
}

saved() {
	[ -f "$DIR/.env" ] || return 0
	sed -n "s/^$1=//p" "$DIR/.env" | tail -n 1
}

set_setting() {
	file="$DIR/.env"
	touch "$file"
	if grep -q "^$1=" "$file"; then
		awk -v key="$1" -v value="$2" 'index($0, key "=") == 1 { print key "=" value; next } { print }' "$file" >"$file.next"
		cat "$file.next" >"$file"
		rm -f "$file.next"
	else
		printf '%s=%s\n' "$1" "$2" >>"$file"
	fi
}

valid_domain() {
	printf '%s' "$1" | grep -Eq '^([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z][a-z0-9-]{0,62}$' || return 1
	! printf '%s' "$1" | grep -Eq '^[0-9.]+$'
}

valid_email() {
	printf '%s' "$1" | grep -Eq '^[^@[:space:]]+@[^@[:space:]]+\.[^@[:space:]]+$'
}

valid_contact() {
	valid_email "$1" || printf '%s' "$1" | grep -Eq '^https://[^[:space:]]+$'
}

valid_version() {
	printf '%s' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'
}

local_name() {
	case "$1" in localhost | *.localhost) return 0 ;; esac
	return 1
}

newest_version() {
	curl -fsS --max-time 20 "$RELEASES" | sed -n 's/^[[:space:]]*"version":[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1
}

fetch() {
	if [ -n "$BUNDLE" ]; then
		cp "$BUNDLE/$1" "$2"
	else
		curl -fsSL --max-time 60 -o "$2" "$FORGE/raw/tag/v$version/deploy/$1"
	fi
}

need_tools() {
	command -v docker >/dev/null || fail "Docker is not installed (see https://docs.docker.com/engine/install/)"
	docker compose version >/dev/null 2>&1 || fail "Docker Compose v2 is not installed"
	docker info >/dev/null 2>&1 || fail "cannot reach Docker; run this as root or as a user in the docker group"
	command -v curl >/dev/null || fail "curl is not installed"
}

pick_settings() {
	[ -n "$domain" ] || domain="$(saved SYNC_DOMAIN)"
	[ -n "$email" ] || email="$(saved ACME_EMAIL)"
	[ -n "$contact" ] || contact="$(saved SYNC_ABUSE_CONTACT)"
	[ -n "$domain" ] || domain="$(ask "Domain for this sync server (for example sync.example.org)" "--domain")"
	domain="$(printf '%s' "$domain" | tr '[:upper:]' '[:lower:]' | sed 's#^https\{0,1\}://##; s#/.*$##; s#\.$##')"
	valid_domain "$domain" || fail "'$domain' is not a domain name"
	[ -n "$email" ] || email="$(ask "Email for certificate notices from Let's Encrypt" "--email")"
	valid_email "$email" || fail "'$email' is not an email address"
	[ -n "$contact" ] || contact="$(ask "Abuse contact people can report to (email or https:// page)" "--contact")"
	valid_contact "$contact" || fail "'$contact' must be an email address or an https:// page"
	if [ -z "$admin" ]; then
		if [ -n "$(saved SYNC_ADMIN_PASSWORD)" ]; then
			admin=on
		elif [ ! -f "$DIR/.env" ] && interactive && confirm "Turn on the admin page? It shows health and certificates, and opens only through an SSH tunnel"; then
			admin=on
		else
			admin=off
		fi
	fi
	case "$admin" in on | off) ;; *) fail "COMMY_ADMIN must be on or off" ;; esac
	case "$admin_port" in *[!0-9]* | "") fail "COMMY_ADMIN_PORT must be a port number" ;; esac
	if [ -z "$version" ]; then
		version="$(newest_version)" || true
		[ -n "$version" ] || fail "could not read the newest release from $RELEASES"
	fi
	version="${version#v}"
	valid_version "$version" || fail "'$version' is not a release version"
}

hand_off_to_release_script() {
	[ -z "$BUNDLE" ] && [ -z "${COMMY_SETUP_HANDOFF:-}" ] || return 0
	next="$(mktemp)"
	fetch sync-host/setup.sh "$next" || fail "release $version has no sync host setup script"
	if [ -f "$0" ] && cmp -s "$0" "$next"; then
		rm -f "$next"
		return 0
	fi
	say "Running the setup script from release $version"
	COMMY_SETUP_HANDOFF=1 SYNC_DOMAIN="$domain" ACME_EMAIL="$email" SYNC_ABUSE_CONTACT="$contact" \
		COMMY_VERSION="$version" COMMY_DIR="$DIR" COMMY_YES="$YES" COMMY_ADMIN="$admin" COMMY_ADMIN_PORT="$admin_port" exec sh "$next"
}

check_dns() {
	local_name "$domain" && return 0
	addresses="$(getent ahosts "$domain" 2>/dev/null | awk '{ print $1 }' | sort -u)"
	[ -n "$addresses" ] || fail "$domain does not resolve yet; add an A (and AAAA) record pointing at this server, then run this again"
	mine="$( (hostname -I 2>/dev/null || true; ip -o addr show 2>/dev/null | awk '{ sub("/.*", "", $4); print $4 }') | tr ' ' '\n' | sort -u)"
	for address in $addresses; do
		printf '%s\n' "$mine" | grep -qx "$address" && return 0
	done
	say "Note: $domain points at $(echo "$addresses" | tr '\n' ' ')which is not an address of this machine."
	say "That is fine behind a router that forwards TCP 80 and 443 and UDP $relay_port here; otherwise fix the DNS record first."
	confirm "Continue anyway?" || fail "stopped so the DNS record can be fixed"
}

check_ports() {
	if command -v ss >/dev/null && [ -z "$(docker ps -q --filter "publish=$relay_port/udp")" ] && ss -lunH "sport = :$relay_port" | grep -q .; then
		fail "UDP port $relay_port is already used by another program; the relay needs it"
	fi
	for port in 80 443; do
		owner="$(docker ps --filter "publish=$port" --format '{{.Names}}' | grep -v '^commy-sync-' || true)"
		[ -z "$owner" ] || fail "port $port is already used by container(s): $owner"
		if command -v ss >/dev/null && [ -z "$(docker ps -q --filter "publish=$port")" ] && ss -ltnH "sport = :$port" | grep -q .; then
			fail "port $port is already used by a program outside Docker (for example a web server); stop it first"
		fi
	done
}

write_bundle() {
	mkdir -p "$DIR"
	for file in compose.yml Caddyfile setup.sh; do
		fetch "sync-host/$file" "$DIR/$file.next" || fail "could not fetch $file for release $version"
		mv "$DIR/$file.next" "$DIR/$file"
	done
	if [ "$VERIFY" = on ] && [ -z "${COMMY_COSIGN_KEY:-}" ]; then
		fetch agent/cosign.pub "$DIR/cosign.pub" || fail "could not fetch the Commy signing key"
	fi
}

pull_and_verify() {
	image="$IMAGES/commy-$1:$version"
	say "Fetching $image"
	docker pull -q "$image" >/dev/null || fail "could not fetch $image"
	pinned="$(docker image inspect -f '{{range .RepoDigests}}{{println .}}{{end}}' "$image" | grep -m 1 "^$IMAGES/commy-$1@" || true)"
	[ -n "$pinned" ] || fail "$image has no registry digest"
	if [ "$VERIFY" = on ]; then
		key="${COMMY_COSIGN_KEY:-$DIR/cosign.pub}"
		docker run --rm -v "$key:/cosign.pub:ro" "$COSIGN_IMAGE" verify --key /cosign.pub -a "release=$version" "$pinned" >/dev/null 2>&1 \
			|| fail "$pinned is not signed by Commy for release $version; nothing was changed"
		say "Signature checked: $pinned"
	else
		say "Skipping the signature check (COMMY_VERIFY=$VERIFY)"
	fi
}

pull_images() {
	pull_and_verify relay
	relay_image="$pinned"
	pull_and_verify sync
	sync_image="$pinned"
}

write_settings() {
	umask 077
	mkdir -p "$DIR"
	touch "$DIR/.env"
	chmod 600 "$DIR/.env"
	[ -n "$(saved POSTGRES_PASSWORD)" ] || set_setting POSTGRES_PASSWORD "$(od -An -tx1 -N32 /dev/urandom | tr -d ' \n')"
	set_setting COMPOSE_PROJECT_NAME commy-sync
	set_setting SYNC_DOMAIN "$domain"
	set_setting ACME_EMAIL "$email"
	set_setting SYNC_ABUSE_CONTACT "$contact"
	set_setting COMMY_VERSION "$version"
	set_setting SYNC_IMAGE "$sync_image"
	set_setting RELAY_IMAGE "$relay_image"
	if [ "$admin" = on ]; then
		[ -n "$(saved SYNC_ADMIN_PASSWORD)" ] || set_setting SYNC_ADMIN_PASSWORD "$(od -An -tx1 -N16 /dev/urandom | tr -d ' \n')"
		set_setting CADDY_ADMIN ":2019"
	else
		set_setting SYNC_ADMIN_PASSWORD ""
		set_setting CADDY_ADMIN ""
	fi
	set_setting ADMIN_PORT "$admin_port"
	mkdir -p "$DIR/custom-tls"
	docker run --rm --user 0 --network none -v "$DIR/custom-tls:/custom-tls" --entrypoint sh "$sync_image" \
		-c 'chown commy:0 /custom-tls && chmod 2750 /custom-tls' || fail "could not prepare $DIR/custom-tls"
}

admin_summary() {
	if [ "$admin" = on ]; then
		cat <<EOF
  Admin page: opens only on this machine. From your own computer run
                ssh -L $admin_port:127.0.0.1:$admin_port you@$domain
              then open http://localhost:$admin_port
              The password is SYNC_ADMIN_PASSWORD in $DIR/.env
EOF
	else
		say "  Admin page: off (turn it on with: sh $DIR/setup.sh --admin)"
	fi
}

start() {
	(cd "$DIR" && docker compose up -d --remove-orphans --quiet-pull) || fail "Docker Compose could not start the sync server"
}

answers() {
	if local_name "$domain"; then
		set -- -k --resolve "$domain:443:127.0.0.1" "https://$domain$1"
	else
		set -- "https://$domain$1"
	fi
	curl -fs --max-time 5 -o /dev/null -w '%{http_code}' "$@" 2>/dev/null
}

healthy() {
	[ "$(answers /sync/healthz)" = 200 ] && [ "$(answers /generate_204)" = 204 ]
}

wait_until_healthy() {
	say "Waiting for the sync server and relay at https://$domain (the first certificate can take a minute)"
	waited=0
	until healthy; do
		if [ "$waited" -ge "$HEALTH_TIMEOUT" ]; then
			(cd "$DIR" && docker compose ps -a && docker compose logs --tail 30) >&2 || true
			fail "https://$domain did not answer within $HEALTH_TIMEOUT seconds; check that TCP 80 and 443 reach this machine and that $domain points here"
		fi
		sleep 5
		waited=$((waited + 5))
	done
}

self_check() {
	set -- --network host
	roots=""
	if local_name "$domain"; then
		(cd "$DIR" && docker compose exec -T caddy cat /data/caddy/pki/authorities/local/root.crt) >"$DIR/local-root.crt" \
			|| fail "could not read Caddy's local certificate authority"
		chmod 644 "$DIR/local-root.crt"
		set -- "$@" --add-host "$domain:127.0.0.1" -v "$DIR/local-root.crt:/local-root.crt:ro"
		roots="--ca /local-root.crt"
	fi
	# shellcheck disable=SC2086
	docker run --rm "$@" --entrypoint commy-sync "$pinned" check "wss://$domain/sync" $roots \
		|| fail "the sync server is up but a test client could not store and read an entry"
}

main() {
	need_tools
	pick_settings
	hand_off_to_release_script
	check_dns
	check_ports
	write_bundle
	pull_images
	write_settings
	start
	wait_until_healthy
	self_check

	cat <<EOF

Your Commy sync server is running release $version.

  In Commy, open Server Settings > Sync Server and enter:  https://$domain

  Open these on your firewall: TCP 80 and 443, UDP $relay_port (the relay for voice,
  streams and files of communities on this server).

  Settings:   $DIR/.env (keep it private, it holds the database password)
  Logs:       cd $DIR && docker compose logs -f sync
  Update:     sh $DIR/setup.sh

EOF
	admin_summary
}

main </dev/null
