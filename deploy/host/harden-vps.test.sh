#!/bin/sh

here="$(cd "$(dirname "$0")" && pwd)"
COMMY_HARDEN_LIB=1
. "$here/harden-vps.sh"
set +eu

FAKE="$(mktemp -d)"
BIN="$(mktemp -d)"
SSHD_CONFIG="$FAKE/sshd_config"
SSHD_DROPIN="$FAKE/sshd_config.d/10-commy.conf"
FAIL2BAN_JAIL="$FAKE/jail.d/commy-sshd.conf"
APT_PERIODIC="$FAKE/apt/20auto-upgrades"
failures=0

for cmd in apt-get fail2ban-server unattended-upgrade; do
	printf '#!/bin/sh\necho "%s $*" >>"%s/calls"\n' "$cmd" "$FAKE" >"$BIN/$cmd"
	chmod +x "$BIN/$cmd"
done
PATH="$BIN:$PATH"

reset() {
	rm -rf "${FAKE:?}"/*
	mkdir -p "$FAKE/home/alice/.ssh" "$FAKE/home/root/.ssh"
	echo "Include $FAKE/sshd_config.d/*.conf" >"$SSHD_CONFIG"
	: >"$FAKE/calls"
	unset SUDO_USER
}

getent() { echo "$2:x:1000:1000::$FAKE/home/$2:/bin/sh"; }
sshd() {
	echo "sshd $*" >>"$FAKE/calls"
	case "$1" in
	-t) [ ! -f "$FAKE/sshd_rejects" ] ;;
	esac
}
systemctl() { echo "systemctl $*" >>"$FAKE/calls"; }

check() {
	if [ "$2" = "$3" ]; then
		echo "ok   - $1"
	else
		echo "FAIL - $1: expected [$3] got [$2]"
		failures=$((failures + 1))
	fi
}

count() { grep -c -- "$1" "$FAKE/calls" | tr -d ' '; }
dropin() { [ -f "$SSHD_DROPIN" ] && tr '\n' ' ' <"$SSHD_DROPIN" | sed 's/ $//'; }

reset
SUDO_USER=alice
out="$(harden_ssh)"
check "no key: refuses" "$?" "1"
check "no key: no drop-in written" "$(dropin)" ""
check "no key: nothing reloaded" "$(count 'systemctl')" "0"
check "no key: says how to add one" "$(echo "$out" | grep -c 'ssh-copy-id alice@')" "1"

reset
SUDO_USER=alice
echo "# ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGxQ alice@laptop" >"$FAKE/home/alice/.ssh/authorized_keys"
harden_ssh >/dev/null
check "commented key: refuses" "$?" "1"
check "commented key: no drop-in" "$(dropin)" ""

reset
SUDO_USER=alice
echo "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGxQ alice@laptop" >"$FAKE/home/alice/.ssh/authorized_keys"
harden_ssh >/dev/null
check "sudo user with key: succeeds" "$?" "0"
check "sudo user with key: drop-in" "$(dropin)" "PasswordAuthentication no KbdInteractiveAuthentication no PermitRootLogin no"
check "sudo user with key: config tested" "$(count 'sshd -t')" "1"
check "sudo user with key: reloaded once" "$(count 'systemctl reload ssh')" "1"
check "sudo user with key: drop-in is world readable" "$(stat -c %a "$SSHD_DROPIN")" "644"

reset
SUDO_USER=alice
echo 'from="203.0.113.0/24",no-agent-forwarding ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ== alice' >"$FAKE/home/alice/.ssh/authorized_keys"
harden_ssh >/dev/null
check "key with options: succeeds" "$?" "0"

reset
echo "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ== root@box" >"$FAKE/home/root/.ssh/authorized_keys"
harden_ssh >/dev/null
check "root key: succeeds" "$?" "0"
check "root key: root stays key-only" "$(dropin)" "PasswordAuthentication no KbdInteractiveAuthentication no PermitRootLogin prohibit-password"

reset
SUDO_USER=alice
echo "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ== root@box" >"$FAKE/home/root/.ssh/authorized_keys"
harden_ssh >/dev/null
check "sudo user without key, root with key: root stays key-only" "$(dropin)" "PasswordAuthentication no KbdInteractiveAuthentication no PermitRootLogin prohibit-password"

reset
SUDO_USER=alice
echo "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGxQ alice@laptop" >"$FAKE/home/alice/.ssh/authorized_keys"
touch "$FAKE/sshd_rejects"
harden_ssh >/dev/null
check "sshd -t failure: aborts" "$?" "1"
check "sshd -t failure: drop-in removed" "$(dropin)" ""
check "sshd -t failure: nothing reloaded" "$(count 'systemctl')" "0"

reset
SUDO_USER=alice
echo "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGxQ alice@laptop" >"$FAKE/home/alice/.ssh/authorized_keys"
mkdir -p "$(dirname "$SSHD_DROPIN")"
echo "PermitRootLogin prohibit-password" >"$SSHD_DROPIN"
touch "$FAKE/sshd_rejects"
harden_ssh >/dev/null
check "sshd -t failure: previous drop-in restored" "$(dropin)" "PermitRootLogin prohibit-password"

reset
SUDO_USER=alice
echo "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGxQ alice@laptop" >"$FAKE/home/alice/.ssh/authorized_keys"
harden_ssh >/dev/null
: >"$FAKE/calls"
harden_ssh >/dev/null
check "re-run: succeeds" "$?" "0"
check "re-run: not reloaded again" "$(count 'systemctl')" "0"
check "re-run: drop-in kept" "$(dropin)" "PasswordAuthentication no KbdInteractiveAuthentication no PermitRootLogin no"

reset
SUDO_USER=alice
echo "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGxQ alice@laptop" >"$FAKE/home/alice/.ssh/authorized_keys"
echo "Port 22" >"$SSHD_CONFIG"
harden_ssh >/dev/null
check "no Include: refuses" "$?" "1"
check "no Include: no drop-in" "$(dropin)" ""

reset
setup_fail2ban >/dev/null
check "fail2ban: jail enables sshd" "$(grep -c '^enabled = true$' "$FAIL2BAN_JAIL")" "1"
check "fail2ban: jail reads the journal" "$(grep -c '^backend = systemd$' "$FAIL2BAN_JAIL")" "1"
check "fail2ban: enabled" "$(count 'systemctl enable --now fail2ban')" "1"
check "fail2ban: restarted after writing the jail" "$(count 'systemctl restart fail2ban')" "1"
check "fail2ban: not installed when present" "$(count 'apt-get install')" "0"
: >"$FAKE/calls"
setup_fail2ban >/dev/null
check "fail2ban: re-run does not restart" "$(count 'systemctl restart fail2ban')" "0"

reset
setup_unattended_upgrades >/dev/null
check "unattended-upgrades: package lists refreshed" "$(grep -c '^APT::Periodic::Update-Package-Lists "1";$' "$APT_PERIODIC")" "1"
check "unattended-upgrades: upgrades on" "$(grep -c '^APT::Periodic::Unattended-Upgrade "1";$' "$APT_PERIODIC")" "1"
check "unattended-upgrades: enabled" "$(count 'systemctl enable --now unattended-upgrades')" "1"

reset
PATH="${PATH#"$BIN":}"
printf '#!/bin/sh\necho "apt-get $*" >>"%s/calls"\n' "$FAKE" >"$FAKE/apt-get"
chmod +x "$FAKE/apt-get"
PATH="$FAKE:$PATH"
setup_fail2ban >/dev/null
setup_unattended_upgrades >/dev/null
check "fail2ban: installed when missing" "$(count 'apt-get install -y -qq fail2ban')" "1"
check "unattended-upgrades: installed when missing" "$(count 'apt-get install -y -qq unattended-upgrades')" "1"
PATH="$BIN:${PATH#"$FAKE":}"

check "no 7842/udp in the default public ports" "$(echo "$PUBLIC_PORTS" | grep -c 7842)" "0"
check "relay ports stay open by default" "$PUBLIC_PORTS" "3478/udp 3479/udp 8443/tcp"

rm -rf "$FAKE" "$BIN"
if [ "$failures" -ne 0 ]; then
	echo "$failures test(s) failed"
	exit 1
fi
echo "all hardening tests passed"
