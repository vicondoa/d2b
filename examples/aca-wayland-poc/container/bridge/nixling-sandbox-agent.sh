#!/usr/bin/env bash
# nixling sandbox agent (ADR 0032, P0 — historical MI relay probe).
#
# Runs inside an Azure Container Apps sandbox. Exposes a Wayland-native app
# over `waypipe server` (SHM-only) and tunnels the byte stream out over an
# Azure Relay hybrid connection using the productionized `nixling-relay`
# sender. Authentication is the sandbox's **managed identity** (plane 2): the
# agent fetches a Microsoft Entra token for https://relay.azure.net from the
# injected IDENTITY_ENDPOINT and hands it to nixling-relay as a bearer — NO
# SAS key ever enters the workload.
#
# Env:
#   NIXLING_RELAY_NS / NIXLING_RELAY_NAMESPACE  Relay namespace FQDN [required]
#   NIXLING_RELAY_ENTITY                        hybrid connection name [required]
#   NIXLING_RELAY_CA   egress-proxy CA (default /etc/ssl/certs/adc-egress-proxy-ca.crt)
#   NIXLING_APP        Wayland app (default: foot)
#   NIXLING_APP_CMD    full app command line (overrides NIXLING_APP)
#   NIXLING_WP_SOCKET  in-container waypipe socket (default /run/nixling/wp.sock)
#   NIXLING_WP_COMPRESS waypipe -c value (default zstd)
set -euo pipefail

NS="${NIXLING_RELAY_NS:-${NIXLING_RELAY_NAMESPACE:-}}"
ENTITY="${NIXLING_RELAY_ENTITY:?NIXLING_RELAY_ENTITY is required}"
[ -n "$NS" ] || { echo "[agent] NIXLING_RELAY_NS is required" >&2; exit 1; }
CA="${NIXLING_RELAY_CA:-/etc/ssl/certs/adc-egress-proxy-ca.crt}"
APP="${NIXLING_APP:-foot}"
APP_CMD="${NIXLING_APP_CMD:-$APP}"
WP_SOCKET="${NIXLING_WP_SOCKET:-/run/nixling/wp.sock}"
WP_COMPRESS="${NIXLING_WP_COMPRESS:-zstd}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/nixling}"
mkdir -p "$XDG_RUNTIME_DIR" "$(dirname "$WP_SOCKET")"
chmod 700 "$XDG_RUNTIME_DIR" || true

log() { printf '[nixling-sandbox-agent] %s\n' "$*" >&2; }

# Fetch a managed-identity Entra token for Azure Relay from the ACA-injected
# IDENTITY_ENDPOINT (App Service MSI style). Pure bash + /dev/tcp (the image
# is minimal). Echoes the access_token.
fetch_mi_token() {
  local ep="${IDENTITY_ENDPOINT:?IDENTITY_ENDPOINT not injected (assign an MI to the sandbox group)}"
  local rest hostport host port path q out
  rest="${ep#http://}"; hostport="${rest%%/*}"; path="/${rest#*/}"
  host="${hostport%%:*}"; port="${hostport##*:}"; [ "$host" = "$port" ] && port=80
  q="?api-version=2019-08-01&resource=https%3A%2F%2Frelay.azure.net"
  exec 3<>"/dev/tcp/$host/$port" || { log "MI endpoint connect failed"; return 1; }
  printf 'GET %s%s HTTP/1.1\r\nHost: %s\r\nX-IDENTITY-HEADER: %s\r\nMetadata: true\r\nConnection: close\r\n\r\n' \
    "$path" "$q" "$host" "${IDENTITY_HEADER:-}" >&3
  out="$(cat <&3)"; exec 3>&-
  # extract "access_token":"..." without jq
  out="${out#*\"access_token\":\"}"; printf '%s' "${out%%\"*}"
}

log "waypipe $(waypipe --version 2>/dev/null | head -1 || echo '?'); app=$APP_CMD"
log "fetching managed-identity Entra token for relay.azure.net ..."
TOKEN="$(fetch_mi_token)"
[ -n "$TOKEN" ] || { log "failed to acquire MI token"; exit 1; }
log "MI token acquired (${#TOKEN} chars); starting nixling-relay sender"

# Start the productionized sender: binds+listens the waypipe socket, then
# dials the relay outbound authenticated by the MI bearer.
NIXLING_RELAY_NAMESPACE="$NS" \
NIXLING_RELAY_ENTITY="$ENTITY" \
NIXLING_RELAY_ENTRA_TOKEN="$TOKEN" \
NIXLING_RELAY_CA_FILE="$CA" \
  nixling-relay sender --target "unix-listen:$WP_SOCKET" &
relay_pid=$!
cleanup() { kill "$relay_pid" 2>/dev/null || true; }
trap cleanup EXIT

# Wait for the sender to bind the socket, then launch waypipe + the app.
for _ in $(seq 1 50); do [ -S "$WP_SOCKET" ] && break; sleep 0.1; done
log "launching waypipe server -- $APP_CMD"
# shellcheck disable=SC2086
exec waypipe --no-gpu -c "$WP_COMPRESS" -s "$WP_SOCKET" server -- $APP_CMD
