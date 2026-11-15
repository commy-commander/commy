#!/bin/sh
FLAG=/flags/relay-paused
BASE_CONFIG=/etc/iroh-relay.toml
TLS_DIR=/run/relay-tls
CADDY_DATA=${CADDY_DATA:-/caddy-data}
CUSTOM_TLS=${CUSTOM_TLS:-/custom-tls}

relay_certificate() {
  [ -n "$RELAY_DOMAIN" ] || return 0
  if [ -f "$CUSTOM_TLS/tls.caddy" ] && [ -f "$CUSTOM_TLS/custom.crt" ] && [ -f "$CUSTOM_TLS/custom.key" ]; then
    echo "$CUSTOM_TLS/custom.crt"
    return 0
  fi
  ls -t "$CADDY_DATA"/caddy/certificates/*/"$RELAY_DOMAIN"/"$RELAY_DOMAIN".crt 2>/dev/null | head -n 1
}

certificate_stamp() {
  cert=$(relay_certificate)
  [ -n "$cert" ] && stat -c '%n %Y' "$cert" "${cert%.crt}.key" 2>/dev/null
}

with_reachable_binds() {
  if [ -s /proc/net/if_inet6 ]; then
    cat "$1"
  else
    sed 's/"\[::\]:/"0.0.0.0:/g' "$1"
  fi
}

access_url_ok() {
  case "$RELAY_ACCESS_URL" in
    http://* | https://*) ;;
    *) return 1 ;;
  esac
  case "$RELAY_ACCESS_URL" in
    *[!A-Za-z0-9:/._-]*) return 1 ;;
  esac
}

with_access_check() {
  cat
  [ -n "$RELAY_ACCESS_URL" ] || return 0
  if ! access_url_ok; then
    echo "[relay] ignoring RELAY_ACCESS_URL: not a plain http(s) address" >&2
    return 0
  fi
  echo
  echo "[access.http]"
  echo "url = \"$RELAY_ACCESS_URL\""
}

plain_config() {
  mkdir -p "$TLS_DIR" && chmod 0755 "$TLS_DIR" || return 1
  with_reachable_binds "$BASE_CONFIG" | with_access_check >"$TLS_DIR/plain.toml"
}

address_discovery_config() {
  cert=$(relay_certificate)
  [ -n "$cert" ] && [ -f "${cert%.crt}.key" ] || return 1
  mkdir -p "$TLS_DIR" && chmod 0755 "$TLS_DIR" || return 1
  rm -f "$TLS_DIR/cert.pem" "$TLS_DIR/key.pem"
  cp "$cert" "$TLS_DIR/cert.pem" && cp "${cert%.crt}.key" "$TLS_DIR/key.pem" || return 1
  chmod 0444 "$TLS_DIR/cert.pem" "$TLS_DIR/key.pem"
  {
    echo "enable_quic_addr_discovery = true"
    cat "$BASE_CONFIG"
    echo
    echo "[tls]"
    echo "cert_mode = \"Manual\""
    echo "manual_cert_path = \"$TLS_DIR/cert.pem\""
    echo "manual_key_path = \"$TLS_DIR/key.pem\""
    echo "quic_bind_addr = \"[::]:7842\""
  } >"$TLS_DIR/full.toml"
  with_reachable_binds "$TLS_DIR/full.toml" | with_access_check >"$TLS_DIR/iroh-relay.toml"
}

run_as_relay() {
  if [ "$(id -u)" = 0 ]; then
    exec setpriv --reuid=relay --regid=relay --clear-groups "$@"
  else
    exec "$@"
  fi
}

while :; do
  if [ -e "$FLAG" ]; then
    sleep 30
    continue
  fi
  stamp=$(certificate_stamp)
  if address_discovery_config; then
    echo "[relay] started with address discovery for $RELAY_DOMAIN"
    run_as_relay iroh-relay --dev --config-path "$TLS_DIR/iroh-relay.toml" &
  else
    echo "[relay] started without address discovery: no certificate for ${RELAY_DOMAIN:-RELAY_DOMAIN} yet"
    plain_config || exit 1
    run_as_relay iroh-relay --config-path "$TLS_DIR/plain.toml" &
  fi
  pid=$!
  while kill -0 "$pid" 2>/dev/null; do
    if [ -e "$FLAG" ]; then
      echo "[relay] paused at the monthly bandwidth limit"
      kill "$pid"
      break
    fi
    if [ "$(certificate_stamp)" != "$stamp" ]; then
      echo "[relay] restarting for a renewed certificate"
      kill "$pid"
      break
    fi
    sleep 10
  done
  wait "$pid" 2>/dev/null
  sleep 2
done
