#!/bin/sh
# Tests for commy-agent.sh against a fake docker CLI. Run: sh deploy/agent/commy-agent.test.sh

here="$(cd "$(dirname "$0")" && pwd)"
AGENT_LIB=1
. "$here/commy-agent.sh"

FAKE="$(mktemp -d)"
BIN="$(mktemp -d)"
STATE_DIR="$FAKE/state"
SERVICES="web"
HEALTH_TIMEOUT=6
POLL=1
REF="ghcr.io/commy/commy-web:dev"
failures=0

# Fake engine. $FAKE/containers holds "id image" lines for the service, $FAKE/tag is
# the image the local tag points at, $FAKE/health is what new containers report.
reset() {
	rm -rf "${FAKE:?}"/*
	echo "c1 sha256:old" >"$FAKE/containers"
	echo "sha256:old" >"$FAKE/tag"
	echo "healthy" >"$FAKE/health"
	echo 1 >"$FAKE/next"
	: >"$FAKE/calls"
	: >"$FAKE/new"
}

new_container() {
	n=$(($(cat "$FAKE/next") + 1))
	echo "$n" >"$FAKE/next"
	echo "c$n $(cat "$FAKE/tag")" >>"$FAKE/containers"
	echo "c$n" >>"$FAKE/new"
}

docker() {
	echo "$*" >>"$FAKE/calls"
	case "$1 $2" in
	"compose ps") cut -d' ' -f1 "$FAKE/containers" ;;
	"compose pull")
		[ -f "$FAKE/pull_fails" ] && return 1
		[ -f "$FAKE/pull_fails_$4" ] && return 1
		[ -f "$FAKE/remote" ] && cp "$FAKE/remote" "$FAKE/tag"
		return 0
		;;
	"compose up")
		case "$*" in
		*--scale*)
			all="$*"
			want="${all##*--scale web=}"
			want="${want%% *}"
			while [ "$(wc -l <"$FAKE/containers" | tr -d ' ')" -lt "$want" ]; do new_container; done
			;;
		*) new_container ;;
		esac
		;;
	"inspect -f")
		id="$4"
		case "$3" in
		*Config.Image*) echo "$REF" ;;
		*State.Health*)
			if grep -qx "$id" "$FAKE/new"; then cat "$FAKE/health"; else echo healthy; fi
			;;
		*) grep "^$id " "$FAKE/containers" | cut -d' ' -f2 ;;
		esac
		;;
	"compose config") echo "$REF" ;;
	"image inspect")
		case "$4" in
		*RepoDigests*) [ -f "$FAKE/nodigest" ] || echo "ghcr.io/commy/commy-web@$(cat "$FAKE/tag")" ;;
		*) cat "$FAKE/tag" ;;
		esac
		;;
	"stop "*) ;;
	"rm -f")
		grep -v "^$3 " "$FAKE/containers" >"$FAKE/c.tmp"
		mv "$FAKE/c.tmp" "$FAKE/containers"
		;;
	esac
}

log() { :; }
sleep() { :; }

cat >"$BIN/cosign" <<EOF
#!/bin/sh
echo "\$*" >>"$FAKE/cosign_calls"
[ ! -f "$FAKE/cosign_fails" ]
EOF
chmod +x "$BIN/cosign"
PATH_WITHOUT_COSIGN="$PATH"
PATH="$BIN:$PATH"

check() {
	if [ "$2" = "$3" ]; then
		echo "ok   - $1"
	else
		echo "FAIL - $1: expected [$3] got [$2]"
		failures=$((failures + 1))
	fi
}

called() {
	grep -c -- "$1" "$FAKE/calls" | tr -d ' '
}

images() {
	cut -d' ' -f2 "$FAKE/containers" | tr '\n' ' ' | sed 's/ $//'
}

# No new image: nothing is started or stopped
reset
run_once
check "unchanged image does not start containers" "$(called 'compose up')" "0"
check "unchanged image keeps the old container" "$(images)" "sha256:old"

# New healthy image: started beside the old one, then the old one is removed
reset
echo "sha256:new" >"$FAKE/remote"
run_once
check "new image is started beside the old one" "$(called 'compose up -d --no-deps --no-build --no-recreate --scale web=2 web')" "1"
check "old container is removed after the swap" "$(called 'rm -f c1')" "1"
check "only the new image is left running" "$(images)" "sha256:new"

# New image never gets healthy: old container keeps serving
reset
echo "sha256:bad" >"$FAKE/remote"
echo "starting" >"$FAKE/health"
run_once
check "slow image reports failure" "$?" "1"
check "slow image leaves the old container running" "$(images)" "sha256:old"

# New unhealthy image: removed, recorded and skipped next cycle
reset
echo "sha256:bad" >"$FAKE/remote"
echo "unhealthy" >"$FAKE/health"
run_once
check "unhealthy image reports failure" "$?" "1"
check "unhealthy container is removed" "$(called 'rm -f c2')" "1"
check "old container never stops" "$(called 'stop -t 30 c1')" "0"
check "old image keeps serving" "$(images)" "sha256:old"
check "bad image is recorded" "$(cat "$STATE_DIR/bad")" "sha256:bad"
: >"$FAKE/calls"
run_once
check "known bad image is skipped" "$(called 'compose up')" "0"

# A newer good image after a bad one: deployed
echo "sha256:fixed" >"$FAKE/remote"
echo "healthy" >"$FAKE/health"
run_once
check "fixed image after a bad one deploys" "$(images)" "sha256:fixed"

# Two replicas: both are replaced
reset
echo "c9 sha256:old" >>"$FAKE/containers"
echo "sha256:new" >"$FAKE/remote"
run_once
check "scale doubles the replica count" "$(called 'scale web=4')" "1"
check "every replica moves to the new image" "$(images)" "sha256:new sha256:new"

# Service not running: started directly
reset
: >"$FAKE/containers"
run_once
check "stopped service is started" "$(called 'compose up -d --no-deps --no-build web')" "1"

# Registry unreachable: nothing touched
reset
touch "$FAKE/pull_fails"
run_once
check "failed pull reports failure" "$?" "1"
check "failed pull does not start containers" "$(called 'compose up')" "0"

# Several services are each checked
reset
SERVICES="auth web"
run_once
check "every watched service is inspected" "$(called 'compose ps -q')" "2"
SERVICES="web"


# Pull retries before giving up
reset
touch "$FAKE/pull_fails"
PULL_TRIES=3
run_once
check "pull is retried" "$(called 'compose pull --quiet web')" "3"

# A failed pull still starts an image that is already local
reset
: >"$FAKE/containers"
touch "$FAKE/pull_fails"
run_once
check "local image starts despite a failed pull" "$(called 'compose up -d --no-deps --no-build web')" "1"

# One service failing to pull does not block the others
reset
SERVICES="auth web"
touch "$FAKE/pull_fails_auth"
echo "sha256:new" >"$FAKE/remote"
run_once
check "other services still update" "$(images)" "sha256:new"
SERVICES="web"

# Signature checks on: a signed image deploys after cosign has seen its digest
reset
VERIFY=on
echo "sha256:new" >"$FAKE/remote"
run_once
check "signed image deploys" "$(images)" "sha256:new"
check "cosign checks the pulled digest, its tag and the promote identity" "$(cat "$FAKE/cosign_calls")" \
	"verify -a tag=dev --certificate-identity $COSIGN_IDENTITY --certificate-oidc-issuer $COSIGN_ISSUER ghcr.io/commy/commy-web@sha256:new"

# Signature missing or wrong: the old container keeps serving and the image is retried next cycle
reset
echo "sha256:new" >"$FAKE/remote"
touch "$FAKE/cosign_fails"
run_once
check "unsigned image reports failure" "$?" "1"
check "unsigned image is not started" "$(called 'compose up')" "0"
check "old image keeps serving when the signature fails" "$(images)" "sha256:old"
check "unsigned image is not recorded as bad" "$([ -f "$STATE_DIR/bad" ] && cat "$STATE_DIR/bad")" ""

# Stopped service: verified before it is started
reset
: >"$FAKE/containers"
touch "$FAKE/cosign_fails"
run_once
check "stopped service stays stopped on an unsigned image" "$(called 'compose up')" "0"
reset
: >"$FAKE/containers"
run_once
check "stopped service starts on a signed image" "$(called 'compose up -d --no-deps --no-build web')" "1"
check "the configured image is what gets verified" "$(grep -c 'ghcr.io/commy/commy-web@sha256:old' "$FAKE/cosign_calls")" "1"

# A locally built image has no registry digest to verify
reset
echo "sha256:new" >"$FAKE/remote"
touch "$FAKE/nodigest"
run_once
check "image without a registry digest is refused" "$?" "1"
check "nothing is verified without a digest" "$(called 'cosign')" "0"
check "old image keeps serving without a digest" "$(images)" "sha256:old"

# cosign missing while checks are on: fail closed
reset
echo "sha256:new" >"$FAKE/remote"
PATH="$PATH_WITHOUT_COSIGN"
run_once
check "missing cosign refuses the image" "$?" "1"
check "missing cosign keeps the old image" "$(images)" "sha256:old"
PATH="$BIN:$PATH_WITHOUT_COSIGN"

# auto: on for prod projects when cosign is installed, off otherwise
VERIFY=auto
COMPOSE_PROJECT_NAME=commy-prod
reset
echo "sha256:new" >"$FAKE/remote"
run_once
check "prod project verifies by default" "$(wc -l <"$FAKE/cosign_calls" | tr -d ' ')" "1"
COMPOSE_PROJECT_NAME=commy-dev
reset
echo "sha256:new" >"$FAKE/remote"
run_once
check "dev project does not verify by default" "$([ -f "$FAKE/cosign_calls" ] && wc -l <"$FAKE/cosign_calls" | tr -d ' ')" ""
check "dev project still deploys" "$(images)" "sha256:new"
COMPOSE_PROJECT_NAME=commy-prod
PATH="$PATH_WITHOUT_COSIGN"
reset
echo "sha256:new" >"$FAKE/remote"
run_once
check "prod project without cosign deploys unverified in auto mode" "$(images)" "sha256:new"
PATH="$BIN:$PATH_WITHOUT_COSIGN"
VERIFY=off
reset
echo "sha256:new" >"$FAKE/remote"
run_once
check "checks can be turned off for prod" "$([ -f "$FAKE/cosign_calls" ] && wc -l <"$FAKE/cosign_calls" | tr -d ' ')" ""
unset COMPOSE_PROJECT_NAME
VERIFY=auto

# Image references
check "repo of a tagged image" "$(repo_of ghcr.io/commy/commy-web:prod)" "ghcr.io/commy/commy-web"
check "repo of a registry with a port" "$(repo_of localhost:5000/commy-web:dev)" "localhost:5000/commy-web"
check "repo of an untagged image on a registry with a port" "$(repo_of localhost:5000/commy-web)" "localhost:5000/commy-web"
check "repo of a digest reference" "$(repo_of ghcr.io/commy/commy-web@sha256:abc)" "ghcr.io/commy/commy-web"
check "tag of a tagged image" "$(tag_of localhost:5000/commy-web:dev)" "dev"
check "tag of an untagged image" "$(tag_of localhost:5000/commy-web)" "latest"
check "tag of a digest reference" "$(tag_of ghcr.io/commy/commy-web@sha256:abc)" ""

# Jitter stays within a third of the interval, capped at 20 seconds
INTERVAL=10
check "short interval keeps jitter small" "$(jitter | awk '{ print ($1 <= 3) }')" "1"
INTERVAL=600
check "jitter is capped at 20s" "$(jitter | awk '{ print ($1 <= 20) }')" "1"

rm -rf "$FAKE" "$BIN"
if [ "$failures" -ne 0 ]; then
	echo "$failures test(s) failed"
	exit 1
fi
echo "all agent tests passed"
