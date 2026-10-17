#!/bin/sh
# End to end test of commy-agent.sh against a real Docker engine and a local registry.
# Run: sh deploy/agent/integration.test.sh   (needs Docker with Compose v2)

set -e
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
port="${REGISTRY_PORT:-5055}"
img="localhost:$port/commy-agent-it:dev"
export COMPOSE_PROJECT_NAME=commy-agent-it
failures=0

cleanup() {
	(cd "$work" && docker compose down -v --remove-orphans >/dev/null 2>&1) || true
	docker rm -f commy-agent-it-registry >/dev/null 2>&1 || true
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

# Publishes a tiny web server that answers with $1; $2 is its healthcheck command.
publish() {
	mkdir -p "$work/img"
	cat >"$work/img/Dockerfile" <<EOF
FROM busybox:1.36
RUN mkdir /www && echo $1 > /www/index.html
HEALTHCHECK --interval=1s --timeout=2s --retries=3 CMD $2
CMD ["httpd", "-f", "-p", "8080", "-h", "/www"]
EOF
	docker build -q -t "$img" "$work/img" >/dev/null
	docker push -q "$img" >/dev/null
}

serving() {
	for id in $(docker compose ps -q web); do
		docker exec "$id" wget -qO- http://127.0.0.1:8080/
	done | tr '\n' ' ' | sed 's/ $//'
}

docker run -d --name commy-agent-it-registry -p "$port:5000" registry:2 >/dev/null
sleep 2
cat >"$work/compose.yml" <<EOF
services:
  web:
    image: $img
EOF
cd "$work"

publish v1 "wget -qO- http://127.0.0.1:8080/"
docker compose up -d --wait >/dev/null 2>&1
check "v1 is serving" "$(serving)" "v1"

AGENT_LIB=1 AGENT_SERVICES=web AGENT_HEALTH_TIMEOUT=60 AGENT_STATE_DIR="$work/state" . "$here/commy-agent.sh"
set +e

publish v2 "wget -qO- http://127.0.0.1:8080/"
run_once
check "healthy update succeeds" "$?" "0"
check "v2 replaced v1" "$(serving)" "v2"
check "one container after the swap" "$(docker compose ps -q web | wc -l | tr -d ' ')" "1"

publish v3 "false"
run_once
check "unhealthy update is rejected" "$?" "1"
check "v2 keeps serving after a bad image" "$(serving)" "v2"
check "one container after the rejection" "$(docker compose ps -q web | wc -l | tr -d ' ')" "1"

run_once
check "bad image is skipped on the next cycle" "$?" "0"
check "v2 still serving" "$(serving)" "v2"

publish v4 "wget -qO- http://127.0.0.1:8080/"
VERIFY=on
run_once
check "unsigned image is refused while signature checks are on" "$?" "1"
check "v2 keeps serving after the refusal" "$(serving)" "v2"
VERIFY=off
run_once
check "the same image deploys with signature checks off" "$?" "0"
check "v4 replaced v2" "$(serving)" "v4"

if [ "$failures" -ne 0 ]; then
	echo "$failures integration test(s) failed"
	exit 1
fi
echo "all agent integration tests passed"
