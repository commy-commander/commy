#!/bin/sh
# Hardens a server set up by setup-vps.sh: ufw with SSH rate limiting, HTTPS only from
# Cloudflare (refreshed daily) and the relay ports open to everyone.
#   sudo sh /srv/commy-prod/deploy/host/harden-vps.sh
# Settings (environment variables):
#   PUBLIC_PORTS   ports open to everyone besides 80, default "3478/udp 3479/udp 7842/udp 8443/tcp" (iroh relays; 3479/udp is the staging relay's STUN)

set -eu

here="$(cd "$(dirname "$0")" && pwd)"
PUBLIC_PORTS="${PUBLIC_PORTS:-3478/udp 3479/udp 7842/udp 8443/tcp}"
CADDY_LOGS="${CADDY_LOGS:-/var/log/commy-caddy}"

[ "$(id -u)" -eq 0 ] || { echo "run with sudo"; exit 1; }

echo "== Firewall"
command -v ufw >/dev/null || { apt-get update -qq && apt-get install -y -qq ufw; }

# SSH first so enabling ufw can never cut off the current session
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
cat >/etc/systemd/system/commy-firewall.service <<'EOF'
[Unit]
Description=Limit HTTPS to Cloudflare for Commy
After=docker.service network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=/usr/local/sbin/commy-firewall apply

[Install]
WantedBy=multi-user.target
EOF
cat >/etc/systemd/system/commy-firewall.timer <<'EOF'
[Unit]
Description=Refresh Cloudflare ranges for Commy daily

[Timer]
OnCalendar=daily
RandomizedDelaySec=1h
Persistent=true

[Install]
WantedBy=timers.target
EOF
# Docker recreates its chains on restart, so re-apply after it
mkdir -p /etc/systemd/system/docker.service.d
cat >/etc/systemd/system/docker.service.d/commy-firewall.conf <<'EOF'
[Service]
ExecStartPost=-/usr/local/sbin/commy-firewall apply
EOF
systemctl daemon-reload
systemctl enable --now commy-firewall.service commy-firewall.timer >/dev/null 2>&1
/usr/local/sbin/commy-firewall apply


# Pick up the current edge Caddyfile and its access log mount
edge="$here/../edge"
if [ -f "$edge/.env" ] && docker compose --project-directory "$edge" ps -q caddy 2>/dev/null | grep -q .; then
	echo "== Edge proxy"
	mkdir -p "$CADDY_LOGS"
	(cd "$edge" && docker compose up -d --force-recreate caddy)
fi

echo "== Status"
ufw status verbose | sed -n '1,4p'
echo "Open to everyone: ${ssh_ports:-22}/tcp (rate limited), 80/tcp, $PUBLIC_PORTS"
echo "443 only from Cloudflare; refreshed daily by commy-firewall.timer"
