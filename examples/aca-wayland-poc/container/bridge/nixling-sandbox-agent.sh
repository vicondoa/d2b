#!/usr/bin/env bash
# nixling sandbox agent (ADR 0032).
#
# Runs inside an Azure Container Apps sandbox. Exposes a Wayland-native app
# over `waypipe server` (SHM-only, no GPU/DMABUF — ACA sandboxes have no
# host GPU) and tunnels the waypipe byte stream out over an Azure Relay
# hybrid connection, where the realm gateway's `waypipe client` renders it
# next to the operator's compositor.
#
# Transport topology (per connection):
#
#   waypipe server -- app
#       --connect--> /run/nixling/wp.sock           (unix socket)
#       <--bind/accept-- nixling-relay-bridge send   (unix-listen target)
#       --WSS(outbound)--> Azure Relay hybrid connection
#       --> nixling-relay-bridge listen (host) --> waypipe client --> niri
#
# The relay bridge binds + listens the unix socket *before* it accepts, so
# `waypipe server` can connect without the bind/listen race that an
# intermediary socat introduces. The agent waits for the socket to appear
# (the bridge is listening by then) and only then launches waypipe.
#
# ACA sandboxes terminate egress TLS with a transparent proxy, so the
# bridge must trust the sandbox proxy CA (NIXLING_RELAY_CA); the webpki /
# Mozilla root set alone yields UnknownIssuer.
#
# The container holds NO long-lived Azure credential: it receives only a
# short-lived, least-privilege Relay *Send* token (NIXLING_RELAY_KEY) minted
# by the gateway — never the full SAS policy key or any provider identity.
# This preserves the ADR 0032 trust boundary (host/realm credentials never
# enter the workload).
#
# Environment:
#   NIXLING_RELAY_NS       Relay namespace FQDN (…servicebus.windows.net)  [required]
#   NIXLING_RELAY_ENTITY   hybrid connection name                          [required]
#   NIXLING_RELAY_KEYNAME  SAS rule name with Send rights                  [required]
#   NIXLING_RELAY_KEY      SAS key (short-lived Send token)                [required]
#   NIXLING_RELAY_CA       sandbox egress-proxy CA bundle
#                          (default: /etc/ssl/certs/adc-egress-proxy-ca.crt)
#   NIXLING_APP            Wayland app to launch          (default: foot)
#   NIXLING_APP_CMD        full shell command to launch instead of NIXLING_APP
#   NIXLING_WP_SOCKET      in-container waypipe unix socket
#                          (default: /run/nixling/wp.sock)
#   NIXLING_WP_COMPRESS    waypipe -c value               (default: zstd)
#   XDG_RUNTIME_DIR        runtime dir                    (default: /run/nixling)
set -euo pipefail

: "${NIXLING_RELAY_NS:?NIXLING_RELAY_NS is required}"
: "${NIXLING_RELAY_ENTITY:?NIXLING_RELAY_ENTITY is required}"
: "${NIXLING_RELAY_KEYNAME:?NIXLING_RELAY_KEYNAME is required}"
: "${NIXLING_RELAY_KEY:?NIXLING_RELAY_KEY is required}"

RELAY_CA="${NIXLING_RELAY_CA:-/etc/ssl/certs/adc-egress-proxy-ca.crt}"
APP="${NIXLING_APP:-foot}"
WP_SOCKET="${NIXLING_WP_SOCKET:-/run/nixling/wp.sock}"
WP_COMPRESS="${NIXLING_WP_COMPRESS:-zstd}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/nixling}"

mkdir -p "$XDG_RUNTIME_DIR" "$(dirname "$WP_SOCKET")"
chmod 700 "$XDG_RUNTIME_DIR" || true
rm -f "$WP_SOCKET"

log() { printf '[nixling-sandbox-agent] %s\n' "$*" >&2; }

log "waypipe $(waypipe --version 2>/dev/null | head -1 || echo '?')"
log "app=${NIXLING_APP_CMD:-$APP} wp_socket=$WP_SOCKET compress=$WP_COMPRESS"
log "relay ns=$NIXLING_RELAY_NS entity=$NIXLING_RELAY_ENTITY ca=$RELAY_CA"

ca_args=()
[ -f "$RELAY_CA" ] && ca_args=(--ca-file "$RELAY_CA")

# 1. The relay bridge binds + listens wp.sock and dials the relay outbound.
nixling-relay-bridge \
  --namespace "$NIXLING_RELAY_NS" \
  --entity "$NIXLING_RELAY_ENTITY" \
  --key-name "$NIXLING_RELAY_KEYNAME" \
  --key "$NIXLING_RELAY_KEY" \
  "${ca_args[@]}" \
  send --target "unix-listen:${WP_SOCKET}" &
bridge_pid=$!

cleanup() {
  log "shutting down"
  kill "$bridge_pid" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# 2. Wait for the bridge to be listening on wp.sock (no bind/listen race).
i=0
while [ ! -S "$WP_SOCKET" ]; do
  if ! kill -0 "$bridge_pid" 2>/dev/null; then
    log "relay bridge exited before opening $WP_SOCKET"
    exit 1
  fi
  i=$((i + 1))
  [ "$i" -gt 300 ] && { log "timed out waiting for $WP_SOCKET"; exit 1; }
  sleep 0.1
done

# 3. Launch the app under waypipe server, connecting to the now-ready socket.
log "launching waypipe --no-gpu -c $WP_COMPRESS -s $WP_SOCKET server -- ${NIXLING_APP_CMD:-$APP}"
if [ -n "${NIXLING_APP_CMD:-}" ]; then
  waypipe --no-gpu -c "$WP_COMPRESS" -s "$WP_SOCKET" server -- sh -lc "$NIXLING_APP_CMD"
else
  waypipe --no-gpu -c "$WP_COMPRESS" -s "$WP_SOCKET" server -- "$APP"
fi
log "waypipe server exited"
