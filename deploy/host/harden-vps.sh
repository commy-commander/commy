#!/bin/sh

set -eu

here="$(cd "$(dirname "$0")" && pwd)"
PUBLIC_PORTS="${PUBLIC_PORTS:-7842/udp 7843/udp 8443/tcp}"
CONSOLE_NET="${CONSOLE_NET-169.254.0.0/16}"
CADDY_LOGS="${CADDY_LOGS:-/var/log/commy-caddy}"
SSHD_CONFIG="${SSHD_CONFIG:-/etc/ssh/sshd_config}"
SSHD_DROPIN="${SSHD_DROPIN:-/etc/ssh/sshd_config.d/10-commy.conf}"
FAIL2BAN_JAIL="${FAIL2BAN_JAIL:-/etc/fail2ban/jail.d/commy-sshd.conf}"
APT_PERIODIC="${APT_PERIODIC:-/etc/apt/apt.conf.d/20auto-upgrades}"

apt_install() {
	DEBIAN_FRONTEND=noninteractive apt-get update -qq
	DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "$@"
}

write_file() {
	if [ -f "$1" ] && [ "$(cat "$1")" = "$2" ]; then
		return 1
	fi
	mkdir -p "$(dirname "$1")"
	printf '%s\n' "$2" >"$1"
	chmod 0644 "$1"
}

setup_firewall() {
	command -v ufw >/dev/null || apt_install ufw

	ssh_ports="$(sshd -T 2>/dev/null | awk '$1 == "port" {print $2}' || true)"
	for port in ${ssh_ports:-22}; do
		ufw limit "$port/tcp" comment "commy-ssh" >/dev/null
	done
	ufw allow 80/tcp comment "commy-http" >/dev/null
	for p in $PUBLIC_PORTS; do
		ufw allow "$p" comment "commy-public" >/dev/null
	done
	ufw default deny incoming >/dev/null
	ufw default allow outgoing >/dev/null
	ufw default allow routed >/dev/null
	ufw --force enable >/dev/null

	install -m 0755 "$here/commy-firewall.sh" /usr/local/sbin/commy-firewall
	cat >/etc/systemd/system/commy-firewall.service <<'UNIT'
[Unit]
Description=Limit HTTPS to Cloudflare for Commy
After=docker.service network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=/usr/local/sbin/commy-firewall apply

[Install]
WantedBy=multi-user.target
UNIT
	cat >/etc/systemd/system/commy-firewall.timer <<'UNIT'
[Unit]
Description=Refresh Cloudflare ranges for Commy daily

[Timer]
OnCalendar=daily
RandomizedDelaySec=1h
Persistent=true

[Install]
WantedBy=timers.target
UNIT
	mkdir -p /etc/systemd/system/docker.service.d
	cat >/etc/systemd/system/docker.service.d/commy-firewall.conf <<'UNIT'
[Service]
ExecStartPost=-/usr/local/sbin/commy-firewall apply
UNIT
	systemctl daemon-reload
	systemctl enable --now commy-firewall.service commy-firewall.timer >/dev/null 2>&1
	/usr/local/sbin/commy-firewall apply
}

home_of() {
	getent passwd "$1" | cut -d: -f6
}

has_ssh_key() {
	home="$(home_of "$1")"
	[ -n "$home" ] || return 1
	for f in "$home/.ssh/authorized_keys" "$home/.ssh/authorized_keys2"; do
		[ -f "$f" ] || continue
		if grep -Ev '^[[:space:]]*#' "$f" | grep -Eq '(^|[[:space:]])(ssh-|ecdsa-|sk-)[A-Za-z0-9@.-]+[[:space:]]+[A-Za-z0-9+/]+=*'; then
			return 0
		fi
	done
	return 1
}

root_login_policy() {
	user="${SUDO_USER:-}"
	if [ -n "$user" ] && [ "$user" != root ] && has_ssh_key "$user"; then
		echo no
	elif has_ssh_key root; then
		echo prohibit-password
	fi
}

reload_sshd() {
	systemctl reload ssh 2>/dev/null || systemctl reload sshd 2>/dev/null || systemctl restart ssh
}

harden_ssh() {
	if ! command -v sshd >/dev/null; then
		echo "sshd is not installed, SSH settings left alone"
		return 0
	fi
	policy="$(root_login_policy)"
	if [ -z "$policy" ]; then
		user="${SUDO_USER:-root}"
		cat <<MSG
No SSH key is installed for $user or root, so password login stays on.
Add your public key first, from your own machine:
  ssh-copy-id $user@<server>
or append it to ~/.ssh/authorized_keys on the server, check that
  ssh $user@<server>
signs in without asking for a password, then run this script again.
MSG
		return 1
	fi
	dropin_dir="$(dirname "$SSHD_DROPIN")"
	config_before="$(cat "$SSHD_CONFIG" 2>/dev/null)"
	new_config="$config_before"
	config_changed=0
	if ! grep -Eq "^[[:space:]]*Include[[:space:]]+$dropin_dir/" "$SSHD_CONFIG" 2>/dev/null; then
		new_config="Include $dropin_dir/*.conf
$new_config"
		config_changed=1
	fi
	if [ -n "$CONSOLE_NET" ] && ! grep -Fq "Match Address $CONSOLE_NET" "$SSHD_CONFIG" 2>/dev/null; then
		new_config="$new_config

Match Address $CONSOLE_NET
	PasswordAuthentication yes
	PermitRootLogin yes"
		config_changed=1
	fi
	wanted="PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin $policy"
	previous=""
	if [ -f "$SSHD_DROPIN" ]; then
		previous="$(cat "$SSHD_DROPIN")"
	fi
	dropin_changed=0
	if write_file "$SSHD_DROPIN" "$wanted"; then
		dropin_changed=1
	fi
	if [ "$dropin_changed" = 0 ] && [ "$config_changed" = 0 ]; then
		echo "SSH: keys only, root login $policy (already set)"
		return 0
	fi
	if [ "$config_changed" = 1 ]; then
		printf '%s\n' "$new_config" >"$SSHD_CONFIG"
	fi
	mkdir -p /run/sshd 2>/dev/null || true
	if ! sshd -t; then
		if [ -n "$previous" ]; then
			printf '%s\n' "$previous" >"$SSHD_DROPIN"
		else
			rm -f "$SSHD_DROPIN"
		fi
		if [ "$config_changed" = 1 ]; then
			printf '%s\n' "$config_before" >"$SSHD_CONFIG"
		fi
		echo "sshd rejected $SSHD_DROPIN: previous settings restored, nothing reloaded"
		return 1
	fi
	reload_sshd
	echo "SSH: password login off, root login $policy ($SSHD_DROPIN)"
	if [ -n "$CONSOLE_NET" ]; then
		echo "SSH: the provider console on $CONSOLE_NET keeps password login"
	fi
}

setup_fail2ban() {
	command -v fail2ban-server >/dev/null || apt_install fail2ban
	jail="[sshd]
enabled = true
backend = systemd
ignoreip = 127.0.0.1/8 ::1${CONSOLE_NET:+ $CONSOLE_NET}
maxretry = 5
findtime = 10m
bantime = 1h"
	systemctl enable --now fail2ban >/dev/null
	if write_file "$FAIL2BAN_JAIL" "$jail"; then
		systemctl restart fail2ban
	fi
	echo "fail2ban: sshd jail on ($FAIL2BAN_JAIL)"
}

setup_unattended_upgrades() {
	command -v unattended-upgrade >/dev/null || apt_install unattended-upgrades
	periodic='APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";'
	write_file "$APT_PERIODIC" "$periodic" || true
	systemctl enable --now unattended-upgrades >/dev/null
	echo "unattended-upgrades: on ($APT_PERIODIC)"
}

setup_edge() {
	edge="$here/../edge"
	if [ -f "$edge/.env" ] && docker compose --project-directory "$edge" ps -q caddy 2>/dev/null | grep -q .; then
		echo "== Edge proxy"
		mkdir -p "$CADDY_LOGS"
		(cd "$edge" && docker compose up -d --force-recreate caddy)
	fi
}

main() {
	[ "$(id -u)" -eq 0 ] || { echo "run with sudo"; exit 1; }
	status=0
	echo "== Firewall"
	setup_firewall
	echo "== SSH"
	harden_ssh || status=1
	echo "== fail2ban"
	setup_fail2ban
	echo "== Unattended upgrades"
	setup_unattended_upgrades
	setup_edge
	echo "== Status"
	ufw status verbose | sed -n '1,4p'
	echo "Open to everyone: ${ssh_ports:-22}/tcp (rate limited), 80/tcp, $PUBLIC_PORTS"
	echo "443 only from Cloudflare; refreshed daily by commy-firewall.timer"
	sshd -T 2>/dev/null | grep -Ei '^(passwordauthentication|kbdinteractiveauthentication|permitrootlogin) ' || true
	return $status
}

if [ "${COMMY_HARDEN_LIB:-0}" != "1" ]; then
	main
fi
