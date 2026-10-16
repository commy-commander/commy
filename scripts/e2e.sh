#!/bin/sh
# End-to-end tests of private invite links, the message doorbell, offline
# delivery and waking a resting device: the desktop app's own code on real iroh
# nodes, against the account server running on a throwaway Postgres. Then the
# edge Caddy checks.
# Run from anywhere: sh scripts/e2e.sh   (needs Postgres binaries, Node 22, Rust)
# An argument picks tests by name, e.g. sh scripts/e2e.sh doorbell

set -e
root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
db_port="${E2E_DB_PORT:-55433}"
auth_port="${E2E_AUTH_PORT:-3911}"

cleanup() {
	if [ -n "$auth_pid" ]; then kill "$auth_pid" 2>/dev/null || true; fi
	if [ -f "$work/pg/postmaster.pid" ]; then
		as_db "$pg_bin/pg_ctl" -D "$work/pg" -m immediate stop >/dev/null 2>&1 || true
	fi
	rm -rf "$work"
}
trap cleanup EXIT

# Postgres refuses to run as root, so a root shell runs it as postgres
as_db() {
	if [ "$(id -u)" = 0 ]; then
		runuser -u postgres -- "$@"
	else
		"$@"
	fi
}

pg_bin="$(pg_config --bindir 2>/dev/null || true)"
if [ ! -x "$pg_bin/initdb" ]; then
	for dir in /usr/lib/postgresql/*/bin; do pg_bin="$dir"; done
fi
[ -x "$pg_bin/initdb" ] || { echo "Postgres server binaries not found"; exit 1; }

echo "starting Postgres on $db_port"
chmod 755 "$work"
mkdir "$work/pg"
[ "$(id -u)" = 0 ] && chown postgres "$work/pg"
as_db "$pg_bin/initdb" -D "$work/pg" -A trust -U commy >/dev/null
as_db "$pg_bin/pg_ctl" -D "$work/pg" -l "$work/pg/log" -w \
	-o "-p $db_port -k $work/pg -c listen_addresses=127.0.0.1" start >/dev/null
database="postgres://commy@127.0.0.1:$db_port/postgres"
# A database from before device names were dropped: the server must start on it
psql "$database" -q -v ON_ERROR_STOP=1 -c 'create table "linkedDevice" (
	"id" text primary key, "nodeId" text not null unique, "name" text not null,
	"userId" text not null, "createdAt" timestamptz not null, "lastSeenAt" timestamptz not null)'

echo "starting the account server on $auth_port"
[ -d "$root/auth/node_modules" ] || (cd "$root/auth" && npm ci --silent)
(
	cd "$root/auth"
	NODE_ENV=production PORT="$auth_port" DATABASE_URL="$database" \
		BETTER_AUTH_URL="http://127.0.0.1:$auth_port" BETTER_AUTH_SECRET="e2e-only-secret-not-used-anywhere-else" \
		exec node src/server.js
) >"$work/auth.log" 2>&1 &
auth_pid=$!
for _ in $(seq 1 100); do
	curl -fs "http://127.0.0.1:$auth_port/healthz" >/dev/null 2>&1 && break
	sleep 0.2
done
curl -fs "http://127.0.0.1:$auth_port/healthz" >/dev/null || { cat "$work/auth.log"; exit 1; }

status=0
COMMY_AUTH_URL="http://127.0.0.1:$auth_port" COMMY_E2E_DATABASE_URL="$database" \
	cargo test --manifest-path "$root/Cargo.toml" -p commy-desktop --features e2e "e2e::${1:-}" -- --nocapture || status=$?
if [ "$status" != 0 ]; then
	echo "--- account server log"
	cat "$work/auth.log"
	exit "$status"
fi

sh "$root/deploy/edge/edge.test.sh"
