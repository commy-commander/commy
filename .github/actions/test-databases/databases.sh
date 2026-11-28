#!/bin/sh
set -eu

dir="$RUNNER_TEMP/test-databases"
export PATH="/opt/homebrew/opt/postgresql@16/bin:$PATH"

free_port() {
  python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}

start() {
  rm -rf "$dir"
  mkdir -p "$dir"
  pg_port="$(free_port)"
  redis_port="$(free_port)"
  initdb -D "$dir/postgres" -U commy --auth=trust -E UTF8 >/dev/null
  pg_ctl -D "$dir/postgres" -l "$dir/postgres.log" -o "-p $pg_port -h 127.0.0.1 -c unix_socket_directories=''" -w start >/dev/null
  createdb -h 127.0.0.1 -p "$pg_port" -U commy commy
  redis-server --port "$redis_port" --bind 127.0.0.1 --dir "$dir" --daemonize yes --save "" --appendonly no \
    --pidfile "$dir/redis.pid" --logfile "$dir/redis.log"
  tries=0
  until redis-cli -p "$redis_port" ping >/dev/null 2>&1; do
    tries=$((tries + 1))
    [ "$tries" -lt 50 ] || { cat "$dir/redis.log"; exit 1; }
    sleep 0.2
  done
  {
    echo "POSTGRES_TEST_URL=postgres://commy:commy@127.0.0.1:$pg_port/commy"
    echo "SYNC_POSTGRES_TEST_URL=postgres://commy:commy@127.0.0.1:$pg_port"
    echo "REDIS_TEST_URL=redis://127.0.0.1:$redis_port"
  } >> "$GITHUB_ENV"
  echo "Postgres on port $pg_port, Redis on port $redis_port"
}

wait_for_exit() {
  tries=0
  while kill -0 "$1" 2>/dev/null && [ "$tries" -lt 100 ]; do
    tries=$((tries + 1))
    sleep 0.1
  done
}

stop() {
  [ -d "$dir" ] || return 0
  if [ -f "$dir/postgres/postmaster.pid" ]; then
    pg_pid="$(head -n 1 "$dir/postgres/postmaster.pid")"
    pg_ctl -D "$dir/postgres" -m immediate -w stop >/dev/null 2>&1 || true
    wait_for_exit "$pg_pid"
  fi
  if [ -f "$dir/redis.pid" ]; then
    redis_pid="$(cat "$dir/redis.pid")"
    kill "$redis_pid" 2>/dev/null || true
    wait_for_exit "$redis_pid"
  fi
  tries=0
  until rm -rf "$dir" 2>/dev/null; do
    tries=$((tries + 1))
    [ "$tries" -lt 20 ] || { echo "Could not remove $dir, the runner clears it later"; return 0; }
    sleep 0.25
  done
}

case "${1:-}" in
  start) start ;;
  stop) stop ;;
  *) echo "usage: databases.sh start|stop" >&2; exit 2 ;;
esac
