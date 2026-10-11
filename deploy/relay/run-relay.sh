#!/bin/sh
# Runs iroh-relay unless relay-meter has paused it for the month. A paused
# relay is fully stopped (STUN included) so clients pick a fallback relay.
FLAG=/flags/relay-paused
while :; do
  if [ -e "$FLAG" ]; then
    sleep 30
    continue
  fi
  iroh-relay --config-path /etc/iroh-relay.toml &
  pid=$!
  echo "[relay] started"
  while kill -0 "$pid" 2>/dev/null; do
    if [ -e "$FLAG" ]; then
      echo "[relay] paused at the monthly bandwidth limit"
      kill "$pid"
      break
    fi
    sleep 10
  done
  wait "$pid" 2>/dev/null
  sleep 2
done
