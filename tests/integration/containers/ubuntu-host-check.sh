#!/usr/bin/env bash
set -euo pipefail

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(cd -- "$HERE/../../.." >/dev/null 2>&1 && pwd)}
export ROOT

# shellcheck source=tests/integration/containers/lib.sh
. "$HERE/lib.sh"

cd "$NLC_ROOT"

if ! command -v nix >/dev/null 2>&1; then
  nlc_log "SKIP: nix unavailable - ubuntu-host-check needs nix to build the static binary"
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
    d2bd_artifact=$(nix build --no-link --print-out-paths \
      "git+file://$NLC_ROOT#packages.${system}.d2bd-guest-static" 2>/dev/null | tail -1) \
      || nlc_fail "could not build packages.$system.d2bd-guest-static"
    broker_artifact=$(nix build --no-link --print-out-paths \
      "git+file://$NLC_ROOT#packages.${system}.d2b-broker-guest-static" 2>/dev/null | tail -1) \
      || nlc_fail "could not build packages.$system.d2b-broker-guest-static"
  else
    nlc_fail "could not build containerImages.$system.ubuntu-host-check"
  fi
fi

if [ -n "${artifact:-}" ] \
  && { [ ! -x "$artifact/bin/d2bd" ] || [ ! -x "$artifact/bin/d2b-broker" ]; }; then
  d2bd_artifact=$(nix build --no-link --print-out-paths \
    "git+file://$NLC_ROOT#packages.${system}.d2bd-guest-static" 2>/dev/null | tail -1) \
    || nlc_fail "could not build packages.$system.d2bd-guest-static"
  broker_artifact=$(nix build --no-link --print-out-paths \
    "git+file://$NLC_ROOT#packages.${system}.d2b-broker-guest-static" 2>/dev/null | tail -1) \
    || nlc_fail "could not build packages.$system.d2b-broker-guest-static"
fi

if [ -n "${d2bd_artifact:-}" ]; then
  [ -x "$d2bd_artifact/bin/d2bd" ] \
    || nlc_fail "d2bd static binary missing from $d2bd_artifact"
  [ -x "$broker_artifact/bin/d2b-broker" ] \
    || nlc_fail "d2b-broker static binary missing from $broker_artifact"
else
  [ -x "$artifact/bin/d2bd" ] \
    || nlc_fail "d2bd static binary missing from $artifact"
  [ -x "$artifact/bin/d2b-broker" ] \
    || nlc_fail "d2b-broker static binary missing from $artifact"
fi

container_name="d2b-ubuntu-hostcheck-$$"
cleanup() {
  "${NLC_PODMAN[@]}" rm -f "$container_name" >/dev/null 2>&1 || true
}
trap cleanup EXIT

set +e
volume_args=()
command='/bin/sh -eu -c '\''cat /etc/os-release; /d2bLib/bin/d2bd --help; /d2bLib/bin/d2b-broker invalid 2>&1 || test $? -eq 2'\'''
if [ -n "${d2bd_artifact:-}" ]; then
  volume_args+=(--volume "$d2bd_artifact:/d2bDaemon:ro")
  volume_args+=(--volume "$broker_artifact:/d2bBroker:ro")
  command='/bin/sh -eu -c '\''cat /etc/os-release; /d2bDaemon/bin/d2bd --help; /d2bBroker/bin/d2b-broker invalid 2>&1 || test $? -eq 2'\'''
else
  volume_args+=(--volume "$artifact:/d2bLib:ro")
fi
output=$("${NLC_PODMAN[@]}" run \
  --rm \
  --name "$container_name" \
  --pull=missing \
  --network none \
  "${volume_args[@]}" \
  docker.io/library/ubuntu:24.04 \
  bash -c "$command" 2>&1)
status=$?
set -e

if [ "$status" -ne 0 ]; then
  nlc_fail "ubuntu-host-check container exited $status: $output"
fi

nlc_log "container output follows"
printf '%s\n' "$output" >&2

nlc_assert_contains "$output" "ID=ubuntu" "os-release"
nlc_assert_contains "$output" 'VERSION_ID="24.04"' "os-release"
nlc_assert_contains "$output" "Usage: d2bd" "d2bd --help"
nlc_assert_contains "$output" "unknown profile: invalid" "d2b-broker profile validation"
nlc_ok "shared d2bd and d2b-broker static binaries execute on Ubuntu 24.04 under rootless podman"
