#!/usr/bin/env bash
# Host side of the ADR 0032 ACA + Wayland forwarding POC.
#
# Brings up the two host processes that receive the sandbox's display:
#   1. `waypipe client` bound to a unix socket next to the operator's
#      compositor (renders forwarded Wayland surfaces).
#   2. `d2b-relay-bridge listen`, which accepts the sandbox's outbound
#      Azure Relay connection and bridges it to the waypipe-client socket.
#
# This is the POC stand-in for the realm gateway's host-side display runner;
# the production design jails both under a broker SpawnRunner role (see the
# plan / ADR 0032). It exists so the live demo is reproducible.
#
# Usage:
#   ./run-host-display.sh up      # start the host display receivers
#   ./run-host-display.sh down    # stop them
#
# Env:
#   D2B_RELAY_NS        Relay namespace FQDN (…servicebus.windows.net)
#   D2B_RELAY_ENTITY    hybrid connection name           (default: hc-d2b-display)
#   D2B_RELAY_KEYNAME   Listen SAS rule name             (default: gateway-listen)
#   D2B_RELAY_KEY       Listen SAS key
#   D2B_RG              resource group                   (default: rg-d2b-centralus)
#   D2B_WP_COMPRESS     waypipe -c value                 (default: zstd)
#   WAYLAND_DISPLAY/XDG_RUNTIME_DIR  the operator's compositor
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RG="${D2B_RG:-rg-d2b-centralus}"
ENTITY="${D2B_RELAY_ENTITY:-hc-d2b-display}"
KEYNAME="${D2B_RELAY_KEYNAME:-gateway-listen}"
COMPRESS="${D2B_WP_COMPRESS:-zstd}"
: "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR must point at the operator runtime dir}"
WPC_SOCK="${XDG_RUNTIME_DIR}/wpc.sock"
PIDFILE="${XDG_RUNTIME_DIR}/d2b-aca-poc-host.pids"

log() { printf '[host-display] %s\n' "$*" >&2; }

bridge_bin() {
  if command -v d2b-relay-bridge >/dev/null 2>&1; then
    command -v d2b-relay-bridge
  else
    echo "$here/../relay-bridge/target/release/d2b-relay-bridge"
  fi
}

down() {
  [ -f "$PIDFILE" ] || { log "no pidfile; nothing to stop"; return 0; }
  while read -r pid; do
    [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
  done < "$PIDFILE"
  rm -f "$PIDFILE" "$WPC_SOCK"
  log "stopped host display receivers"
}

up() {
  : "${D2B_RELAY_NS:?D2B_RELAY_NS is required}"
  local key="${D2B_RELAY_KEY:-}"
  if [ -z "$key" ]; then
    log "fetching $KEYNAME key from $RG via az..."
    key="$(az relay hyco authorization-rule keys list \
      -g "$RG" --namespace-name "${D2B_RELAY_NS%%.*}" \
      --hybrid-connection-name "$ENTITY" --name "$KEYNAME" \
      --query primaryKey -o tsv)"
  fi
  local bin; bin="$(bridge_bin)"
  [ -x "$bin" ] || { log "relay-bridge not built: $bin (cargo build --release in relay-bridge/)"; exit 1; }

  rm -f "$WPC_SOCK"
  : > "$PIDFILE"

  log "starting waypipe client on $WPC_SOCK"
  waypipe --no-gpu -c "$COMPRESS" -s "$WPC_SOCK" client &
  echo $! >> "$PIDFILE"
  sleep 1

  log "starting relay listener (entity=$ENTITY)"
  "$bin" --namespace "$D2B_RELAY_NS" --entity "$ENTITY" \
    --key-name "$KEYNAME" --key "$key" \
    listen --target "unix:$WPC_SOCK" &
  echo $! >> "$PIDFILE"

  log "host ready; the sandbox's forwarded windows will appear on this compositor"
}

case "${1:-up}" in
  up)   up ;;
  down) down ;;
  *)    log "usage: $0 {up|down}"; exit 2 ;;
esac
