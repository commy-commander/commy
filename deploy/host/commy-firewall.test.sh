#!/bin/sh
# Tests for commy-firewall.sh with fake iptables, ufw and curl.
# Run: sh deploy/host/commy-firewall.test.sh

here="$(cd "$(dirname "$0")" && pwd)"
COMMY_FW_LIB=1
. "$here/commy-firewall.sh"
set +eu

FAKE="$(mktemp -d)"
STATE="$FAKE/state"
failures=0

reset() {
	rm -rf "${FAKE:?}"/*
	: >"$FAKE/rules"
	: >"$FAKE/ufw"
	echo "" >"$FAKE/cf"
}

curl() {
	[ -s "$FAKE/cf" ] && [ "$(cat "$FAKE/cf")" != "" ] || return 22
	cat "$FAKE/cf"
}

ip() { echo "default via 1.2.3.1 dev eth9"; }

iptables() {
	case "$1" in
	-L) [ ! -f "$FAKE/no-docker-user" ] ;;
	-N) grep -qx "chain $2" "$FAKE/rules" && return 1; echo "chain $2" >>"$FAKE/rules" ;;
	-F) grep -v "^-A $2 " "$FAKE/rules" >"$FAKE/r.tmp"; mv "$FAKE/r.tmp" "$FAKE/rules" ;;
	-C) shift; grep -qxF -- "-A $*" "$FAKE/rules" ;;
	-A | -I) op="$1"; shift; echo "-A $*" >>"$FAKE/rules"; [ "$op" = -I ] && echo "insert" >>"$FAKE/inserts" ;;
	esac
}
ip6tables() { return 1; }

ufw() {
	case "$1" in
	status)
		n=0
		while read -r line; do n=$((n + 1)); echo "[ $n] $line"; done <"$FAKE/ufw"
		;;
	--force) sed -i "${3}d" "$FAKE/ufw" ;;
	allow) echo "443/tcp ALLOW IN $5 # ${11}" >>"$FAKE/ufw" ;;
	esac
}

check() {
	if [ "$2" = "$3" ]; then
		echo "ok   - $1"
	else
		echo "FAIL - $1: expected [$3] got [$2]"
		failures=$((failures + 1))
	fi
}

count() { grep -c -- "$1" "$2" | tr -d ' '; }

# Range validation
check "accepts IPv4 ranges" "$(valid_ranges v4 '1.2.3.0/24 5.6.0.0/16' && echo y)" "y"
check "accepts IPv6 ranges" "$(valid_ranges v6 '2400:cb00::/32' && echo y)" "y"
check "rejects an HTML error page" "$(valid_ranges v4 '<html>error</html>' || echo n)" "n"
check "rejects an empty response" "$(valid_ranges v4 '' || echo n)" "n"
check "rejects IPv6 in the IPv4 list" "$(valid_ranges v4 '2400:cb00::/32' || echo n)" "n"

# Range source: live, then cached, then builtin
reset
check "falls back to the builtin list offline" "$(ranges v4 u 'builtin')" "builtin"
echo "9.9.9.0/24 8.8.0.0/16" >"$FAKE/cf"
check "uses the live list" "$(ranges v4 u 'builtin')" "9.9.9.0/24 8.8.0.0/16 "
echo "" >"$FAKE/cf"
check "uses the cached list when offline later" "$(ranges v4 u 'builtin')" "9.9.9.0/24 8.8.0.0/16 "
echo "<html>" >"$FAKE/cf"
check "ignores a garbage response" "$(ranges v4 u 'builtin')" "9.9.9.0/24 8.8.0.0/16 "

# DOCKER-USER chain
reset
apply_docker_user iptables "1.1.1.0/24 2.2.0.0/16" eth9
check "allows each Cloudflare range" "$(count 'COMMY-CF -s .* -j RETURN' "$FAKE/rules")" "2"
check "drops everything else" "$(count '^-A COMMY-CF -j DROP$' "$FAKE/rules")" "1"
check "drop comes after the allows" "$(grep '^-A COMMY-CF' "$FAKE/rules" | tail -n 1)" "-A COMMY-CF -j DROP"
check "hooks tcp and udp 443" "$(count 'DOCKER-USER -i eth9 -p .* --ctorigdstport 443 --ctdir ORIGINAL -j COMMY-CF' "$FAKE/rules")" "2"
check "does not touch port 80" "$(count 'ctorigdstport 80' "$FAKE/rules")" "0"
apply_docker_user iptables "1.1.1.0/24" eth9
check "re-apply replaces the ranges" "$(count 'COMMY-CF -s' "$FAKE/rules")" "1"
check "re-apply does not duplicate hooks" "$(count 'DOCKER-USER' "$FAKE/rules")" "2"

reset
touch "$FAKE/no-docker-user"
apply_docker_user iptables "1.1.1.0/24" eth9
check "skips hosts without Docker" "$(wc -l <"$FAKE/rules" | tr -d ' ')" "0"

# ufw rules
reset
echo "22/tcp LIMIT IN Anywhere # commy-ssh" >"$FAKE/ufw"
apply_ufw "1.1.1.0/24 2.2.0.0/16"
check "adds a ufw rule per range" "$(count 'commy-cf' "$FAKE/ufw")" "2"
apply_ufw "3.3.3.0/24"
check "re-apply replaces old ufw rules" "$(count 'commy-cf' "$FAKE/ufw")" "1"
check "keeps unrelated ufw rules" "$(count 'commy-ssh' "$FAKE/ufw")" "1"

# Extra allowed ranges
reset
COMMY_FW_ALLOW="10.0.0.0/8 fd00::/8"
apply >/dev/null
check "extra IPv4 range is allowed" "$(count 'COMMY-CF -s 10.0.0.0/8' "$FAKE/rules")" "1"
unset COMMY_FW_ALLOW

rm -rf "$FAKE"
if [ "$failures" -ne 0 ]; then
	echo "$failures test(s) failed"
	exit 1
fi
echo "all firewall tests passed"
