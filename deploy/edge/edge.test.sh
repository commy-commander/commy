#!/bin/sh
# Runs the edge Caddyfile's site block in a real Caddy and checks two things:
# invite and doorbell requests never reach the access log, and the auth server
# is told the visitor's own address (one value, which its rate limits need)
# whether the visitor comes through a trusted proxy or straight in.
# Run: sh deploy/edge/edge.test.sh   (downloads Caddy unless CADDY is set)

set -e
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
caddy_version=2.10.2
port="${EDGE_TEST_PORT:-3944}"
echo_port="${EDGE_TEST_ECHO_PORT:-3945}"
failures=0

cleanup() {
	if [ -n "$caddy_pid" ]; then kill "$caddy_pid" 2>/dev/null || true; fi
	if [ -n "$echo_pid" ]; then kill "$echo_pid" 2>/dev/null || true; fi
	rm -rf "$work"
}
trap cleanup EXIT

check() {
	if [ "$2" = "$3" ]; then
		echo "ok   - $1"
	else
		echo "FAIL - $1: expected [$3] got [$2]"
		failures=$((failures + 1))
	fi
}

caddy="${CADDY:-}"
if [ -z "$caddy" ]; then
	curl -fsSL -o "$work/caddy.tgz" \
		"https://github.com/caddyserver/caddy/releases/download/v$caddy_version/caddy_${caddy_version}_linux_amd64.tar.gz"
	tar -xzf "$work/caddy.tgz" -C "$work" caddy
	caddy="$work/caddy"
fi

# An upstream that answers with the X-Forwarded-For header it was sent
cat >"$work/echo.mjs" <<'EOF'
import http from "node:http";
http
  .createServer((req, res) => res.end(String(req.headers["x-forwarded-for"] ?? "")))
  .listen(Number(process.argv[2]), "127.0.0.1");
EOF
node "$work/echo.mjs" "$echo_port" &
echo_pid=$!

# The real global options and site snippet. 127.0.0.2 stands in for Cloudflare,
# and every upstream is the echo server.
awk 'NR == 1, /^}/' "$here/Caddyfile" |
	sed -e "s#trusted_proxies static .*#trusted_proxies static 127.0.0.2/32#" \
		-e "s#email {\$ACME_EMAIL}#auto_https off#" >"$work/Caddyfile"
awk '/^\(commy\) \{/ { p = 1 } p { print } p && /^}/ { exit }' "$here/Caddyfile" |
	sed -e "s#{args\[0\]}-auth:3001#127.0.0.1:$echo_port#" \
		-e "s#{args\[0\]}-web:3000#127.0.0.1:$echo_port#" \
		-e "s#/var/log/caddy/access.log#$work/access.log#" >>"$work/Caddyfile"
printf '\nhttp://:%s {\n\timport commy edge-test\n}\n' "$port" >>"$work/Caddyfile"

"$caddy" run --config "$work/Caddyfile" --adapter caddyfile >"$work/caddy.out" 2>&1 &
caddy_pid=$!
for _ in $(seq 1 50); do
	curl -fs "http://127.0.0.1:$port/" >/dev/null 2>&1 && break
	sleep 0.2
done

url="http://127.0.0.1:$port"
via_proxy() {
	curl -fs --interface 127.0.0.2 -H "CF-Connecting-IP: 198.51.100.7" -H "X-Forwarded-For: 198.51.100.7" "$@"
}

check "a visitor through the trusted proxy is named alone" \
	"$(via_proxy "$url/api/auth/get-session")" "198.51.100.7"
check "a direct visitor is named by their address" \
	"$(curl -fs "$url/api/auth/get-session")" "127.0.0.1"
check "a direct visitor cannot claim another address" \
	"$(curl -fs -H "X-Forwarded-For: 6.6.6.6" "$url/api/auth/get-session")" "127.0.0.1"
check "doorbell calls are named the same way" \
	"$(via_proxy -X POST "$url/api/auth/commy/doorbell/ring")" "198.51.100.7"

for path in /i/K7QX2M9P "/api/auth/commy/invites/resolve?code=K7QX2M9P" /api/auth/commy/invites/create \
	/api/auth/commy/doorbell/take /api/auth/commy/doorbell/ring /docs; do
	curl -fs -o /dev/null "$url$path"
done
sleep 1
logged() {
	grep -c "\"uri\":\"$1" "$work/access.log" || true
}
check "invite pages are not logged" "$(logged /i/)" 0
check "invite lookups are not logged" "$(logged /api/auth/commy/invites/)" 0
check "doorbell calls are not logged" "$(logged /api/auth/commy/doorbell/)" 0
check "other requests are logged" "$(logged /docs)" 1

if [ "$failures" -gt 0 ]; then
	echo "$failures check(s) failed"
	cat "$work/caddy.out"
	exit 1
fi
echo "edge checks passed"
