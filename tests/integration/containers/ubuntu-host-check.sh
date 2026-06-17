#!/usr/bin/env bash
set -euo pipefail

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(cd -- "$HERE/../../.." >/dev/null 2>&1 && pwd)}
export ROOT

# shellcheck source=tests/integration/containers/lib.sh
. "$HERE/lib.sh"

cd "$NLC_ROOT"

if ! command -v nix >/dev/null 2>&1; then
  nlc_log "SKIP: nix unavailable — ubuntu-host-check needs nix to build the static binary"
  exit 0
fi

nlc_require_podman

system=$(nix eval --raw --impure --expr builtins.currentSystem)
artifact=""
if ! artifact=$(nlc_build_image ubuntu-host-check); then
  flake_out=$(nix eval --raw --impure --no-warn-dirty \
    --expr "(builtins.getFlake \"git+file://$NLC_ROOT\").outPath" 2>/dev/null || true)
  if [ -n "$flake_out" ] \
    && [ -e "$NLC_ROOT/tests/integration/containers/images/ubuntu-host-check.nix" ] \
    && [ ! -e "$flake_out/tests/integration/containers/images/ubuntu-host-check.nix" ]; then
    nlc_log "containerImages.$system.ubuntu-host-check is absent from the git+file snapshot; using static package output for this uncommitted worktree"
    artifact=$(nix build --no-link --print-out-paths \
      "git+file://$NLC_ROOT#packages.${system}.nixling-guestd-static" 2>/dev/null | tail -1) \
      || nlc_fail "could not build packages.$system.nixling-guestd-static"
  else
    nlc_fail "could not build containerImages.$system.ubuntu-host-check"
  fi
fi

[ -x "$artifact/bin/nixling-guestd" ] \
  || nlc_fail "nixling-guestd static binary missing from $artifact"

container_name="nixling-ubuntu-hostcheck-$$"
cleanup() {
  "${NLC_PODMAN[@]}" rm -f "$container_name" >/dev/null 2>&1 || true
}
trap cleanup EXIT

set +e
output=$("${NLC_PODMAN[@]}" run \
  --rm \
  --name "$container_name" \
  --pull=missing \
  --network none \
  --volume "$artifact:/nl:ro" \
  docker.io/library/ubuntu:24.04 \
  /bin/sh -eu -c 'cat /etc/os-release; /nl/bin/nixling-guestd --version' 2>&1)
status=$?
set -e

if [ "$status" -ne 0 ]; then
  nlc_fail "ubuntu-host-check container exited $status: $output"
fi

nlc_log "container output follows"
printf '%s\n' "$output" >&2

nlc_assert_contains "$output" "ID=ubuntu" "os-release"
nlc_assert_contains "$output" 'VERSION_ID="24.04"' "os-release"
nlc_assert_contains "$output" "nixling-guestd 0.0.0-bootstrap" "nixling-guestd --version"
nlc_ok "nixling-guestd-static executes on Ubuntu 24.04 under rootless podman"
