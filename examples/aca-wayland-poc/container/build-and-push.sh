#!/usr/bin/env bash
# Build the nixling Wayland sandbox image with Nix and push it to the ACR
# created by the Bicep deployment (ADR 0032, Wave P0).
#
# Usage:
#   ./build-and-push.sh [<registry-name>] [<repository:tag>]
#
# Defaults discover the registry from the deployed resource group. ACR has
# admin disabled, so we push with a short-lived AAD token (az acr login
# --expose-token) — no stored registry credentials.
set -euo pipefail

REGION="${NIXLING_REGION:-centralus}"
WORKLOAD="${NIXLING_WORKLOAD:-nixling}"
RG="${NIXLING_RG:-rg-${WORKLOAD}-${REGION}}"

REGISTRY="${1:-}"
IMAGE_REF="${2:-nixling-wayland:latest}"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

log() { printf '[build-and-push] %s\n' "$*" >&2; }

if [ -z "$REGISTRY" ]; then
  log "discovering registry in $RG..."
  REGISTRY="$(az acr list -g "$RG" --query '[0].name' -o tsv)"
fi
[ -n "$REGISTRY" ] || { log "no registry found in $RG; deploy the Bicep first"; exit 1; }
LOGIN_SERVER="$(az acr show -n "$REGISTRY" --query loginServer -o tsv)"
log "registry=$REGISTRY login_server=$LOGIN_SERVER image=$IMAGE_REF"

# 1. Build the OCI image (a gzipped docker-archive) with Nix.
log "building image with Nix (waypipe matches the host version)..."
nix-build "$here/image.nix" --out-link "$here/result" >/dev/null
IMAGE_TAR="$(readlink -f "$here/result")"
log "built $IMAGE_TAR"

# 2. Load it into podman.
log "loading into podman..."
LOADED="$(podman load -i "$IMAGE_TAR" 2>&1 | sed -n 's/.*: //p' | tail -1)"
: "${LOADED:=localhost/nixling-wayland:latest}"
log "loaded as $LOADED"

# 3. Tag for the registry.
TARGET="${LOGIN_SERVER}/${IMAGE_REF}"
podman tag "$LOADED" "$TARGET"

# 4. Push with a short-lived AAD token (admin user is disabled on the ACR).
log "logging in to $LOGIN_SERVER with an AAD token..."
TOKEN="$(az acr login -n "$REGISTRY" --expose-token --query accessToken -o tsv)"
echo "$TOKEN" | podman login "$LOGIN_SERVER" \
  --username 00000000-0000-0000-0000-000000000000 --password-stdin

log "pushing $TARGET ..."
podman push "$TARGET"
log "done: $TARGET"
