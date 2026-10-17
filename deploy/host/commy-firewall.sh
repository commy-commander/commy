#!/bin/sh

set -eu

# shellcheck disable=SC1091
[ -f /etc/default/commy-firewall ] && . /etc/default/commy-firewall

STATE="${COMMY_FW_STATE:-/var/lib/commy-firewall}"
CHAIN="COMMY-CF"
TAG="commy-cf"

BUILTIN_V4="173.245.48.0/20 103.21.244.0/22 103.22.200.0/22 103.31.4.0/22 141.101.64.0/18 108.162.192.0/18 190.93.240.0/20 188.114.96.0/20 197.234.240.0/22 198.41.128.0/17 162.158.0.0/15 104.16.0.0/13 104.24.0.0/14 172.64.0.0/13 131.0.72.0/22"
BUILTIN_V6="2400:cb00::/32 2606:4700::/32 2803:f800::/32 2405:b500::/32 2405:8100::/32 2a06:98c0::/29 2c0f:f248::/32"

ranges() {
	family="$1" url="$2" builtin="$3"
	mkdir -p "$STATE"
	fresh="$(curl -fsS --max-time 15 "$url" 2>/dev/null | tr -s ' \r\n' ' ' || true)"
	if valid_ranges "$family" "$fresh"; then
		echo "$fresh" >"$STATE/$family"
	fi
	if [ -s "$STATE/$family" ]; then cat "$STATE/$family"; else echo "$builtin"; fi
}

valid_ranges() {
	[ -n "$2" ] || return 1
	for r in $2; do
		case "$1:$r" in
		v4:*.*.*.*/*) ;;
		v6:*:*/*) ;;
		*) return 1 ;;
		esac
	done
}

apply_docker_user() {
	ipt="$1" list="$2" iface="$3"
	$ipt -L DOCKER-USER -n >/dev/null 2>&1 || return 0
	$ipt -N "$CHAIN" 2>/dev/null || $ipt -F "$CHAIN"
	for r in $list; do
		$ipt -A "$CHAIN" -s "$r" -j RETURN
	done
	$ipt -A "$CHAIN" -j DROP
	for proto in tcp udp; do
		rule="-i $iface -p $proto -m conntrack --ctorigdstport 443 --ctdir ORIGINAL -j $CHAIN"
		# shellcheck disable=SC2086
		$ipt -C DOCKER-USER $rule 2>/dev/null || $ipt -I DOCKER-USER $rule
	done
}

apply_ufw() {
	command -v ufw >/dev/null || return 0
	ufw status numbered | grep "$TAG" | sed -n 's/^\[ *\([0-9]*\)\].*/\1/p' | sort -rn |
		while read -r n; do ufw --force delete "$n" >/dev/null; done
	for r in $1; do
		ufw allow proto tcp from "$r" to any port 443 comment "$TAG" >/dev/null
	done
}

apply() {
	v4="$(ranges v4 https://www.cloudflare.com/ips-v4 "$BUILTIN_V4")"
	v6="$(ranges v6 https://www.cloudflare.com/ips-v6 "$BUILTIN_V6")"
	for r in ${COMMY_FW_ALLOW:-}; do
		case "$r" in *:*) v6="$v6 $r" ;; *) v4="$v4 $r" ;; esac
	done
	iface="${COMMY_FW_IFACE:-$(ip route show default | awk '{print $5; exit}')}"
	apply_docker_user iptables "$v4" "$iface"
	apply_docker_user ip6tables "$v6" "$iface"
	apply_ufw "$v4 $v6"
	echo "commy-firewall: 443 limited to $(echo "$v4 $v6" | wc -w | tr -d ' ') Cloudflare ranges on $iface"
}

remove() {
	for ipt in iptables ip6tables; do
		for proto in tcp udp; do
			while $ipt -D DOCKER-USER -p "$proto" -m conntrack --ctorigdstport 443 --ctdir ORIGINAL -j "$CHAIN" -i "${COMMY_FW_IFACE:-$(ip route show default | awk '{print $5; exit}')}" 2>/dev/null; do :; done
		done
		$ipt -F "$CHAIN" 2>/dev/null || true
		$ipt -X "$CHAIN" 2>/dev/null || true
	done
	apply_ufw ""
	echo "commy-firewall: removed"
}

if [ "${COMMY_FW_LIB:-0}" != "1" ]; then
	case "${1:-apply}" in
	apply) apply ;;
	remove) remove ;;
	*) echo "usage: commy-firewall.sh [apply|remove]"; exit 2 ;;
	esac
fi
