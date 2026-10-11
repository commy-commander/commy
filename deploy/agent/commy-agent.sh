#!/bin/sh
# Commy deploy agent. Polls the registry for the watched services and swaps in new
# images without downtime: a new container starts beside the old one, and the old one
# is removed only once the new one is healthy. Unhealthy images are dropped and skipped.
# Runs inside the docker:cli image with the host Docker socket mounted.

SERVICES="${AGENT_SERVICES:-auth web}"
INTERVAL="${AGENT_INTERVAL:-60}"
HEALTH_TIMEOUT="${AGENT_HEALTH_TIMEOUT:-180}"
STATE_DIR="${AGENT_STATE_DIR:-/tmp/commy-agent}"
POLL="${AGENT_HEALTH_POLL:-3}"
PULL_TRIES="${AGENT_PULL_TRIES:-3}"

log() {
	echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) $*"
}

is_bad() {
	[ -f "$STATE_DIR/bad" ] && grep -qx "$1" "$STATE_DIR/bad"
}

mark_bad() {
	mkdir -p "$STATE_DIR"
	echo "$1" >>"$STATE_DIR/bad"
}

# Waits until every given container reports healthy (or running, without a healthcheck).
wait_healthy() {
	waited=0
	while [ "$waited" -lt "$HEALTH_TIMEOUT" ]; do
		ready=1
		for id in "$@"; do
			state="$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "$id" 2>/dev/null)"
			case "$state" in
			healthy | running) ;;
			unhealthy | exited | dead | "") return 1 ;;
			*) ready=0 ;;
			esac
		done
		[ "$ready" -eq 1 ] && return 0
		sleep "$POLL"
		waited=$((waited + POLL))
	done
	return 1
}

remove() {
	for id in "$@"; do
		docker stop -t 30 "$id" >/dev/null 2>&1
		docker rm -f "$id" >/dev/null 2>&1
	done
}

# Brings one service up to its latest pulled image. Returns 0 when nothing changed
# or the swap went healthy, 1 when the new image was rejected.
update_service() {
	svc="$1"
	old="$(docker compose ps -q "$svc" 2>/dev/null)"
	if [ -z "$old" ]; then
		log "$svc: not running, starting it"
		docker compose up -d --no-deps --no-build "$svc"
		return 0
	fi

	first="$(echo "$old" | head -n 1)"
	ref="$(docker inspect -f '{{.Config.Image}}' "$first")"
	running="$(docker inspect -f '{{.Image}}' "$first")"
	latest="$(docker image inspect -f '{{.Id}}' "$ref" 2>/dev/null)"

	if [ -z "$latest" ] || [ "$latest" = "$running" ] || is_bad "$latest"; then
		return 0
	fi

	count="$(echo "$old" | wc -l | tr -d ' ')"
	log "$svc: starting $ref ($latest) beside $count running"
	docker compose up -d --no-deps --no-build --no-recreate --scale "$svc=$((count * 2))" "$svc"
	new="$(docker compose ps -q "$svc" | grep -vxF "$old")"

	# shellcheck disable=SC2086
	if [ -n "$new" ] && wait_healthy $new; then
		# shellcheck disable=SC2086
		remove $old
		log "$svc: now on $latest"
		return 0
	fi

	log "$svc: $latest failed its health check, keeping $running"
	mark_bad "$latest"
	# shellcheck disable=SC2086
	remove $new
	return 1
}

# Pulls one service, retrying: concurrent pulls of shared layers (several agents on
# one host) can fail transiently with "unable to lease content" on containerd.
pull_service() {
	tries=0
	while [ "$tries" -lt "$PULL_TRIES" ]; do
		docker compose pull --quiet "$1" && return 0
		tries=$((tries + 1))
		sleep "$((tries * 5))"
	done
	return 1
}

run_once() {
	status=0
	for svc in $SERVICES; do
		if ! pull_service "$svc"; then
			log "$svc: pull failed, retrying next cycle"
			status=1
		fi
		# Still reconcile: an image pulled earlier (or by hand) may be waiting
		update_service "$svc" || status=1
	done
	return $status
}

main() {
	log "watching [$SERVICES] in project ${COMPOSE_PROJECT_NAME:-commy} every ${INTERVAL}s"
	while true; do
		run_once
		docker image prune -f >/dev/null 2>&1
		# Jitter keeps agents on the same host from pulling in lockstep
		sleep "$((INTERVAL + $(awk 'BEGIN { srand(); print int(rand() * 20) }')))"
	done
}

if [ "${AGENT_LIB:-0}" != "1" ]; then
	main
fi
