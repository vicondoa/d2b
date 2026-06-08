#!/usr/bin/env bash
# nixling-ipc wire types -> daemon-api.md drift.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

cd "$ROOT"

if [ ! -f packages/xtask/Cargo.toml ] || [ ! -f docs/reference/daemon-api.md ]; then
  log "daemon-api-drift inputs absent — skipping"
  exit 0
fi

if [ -z "${NIXLING_DAEMON_API_DRIFT_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "daemon-api-drift: neither cargo nor nix is on PATH"
    exit 1
  fi
  log "  cargo not on PATH; re-entering via nix shell to acquire toolchain"
  export NIXLING_DAEMON_API_DRIFT_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

xtask_bin=$(nl_cargo_bin_path workspace xtask)
if [ ! -x "$xtask_bin" ]; then
  (
    cd packages
    CARGO_TARGET_DIR="$(nl_cargo_target_dir workspace)" cargo build -q --manifest-path "$ROOT/packages/Cargo.toml" -p xtask --bin xtask
  )
fi

log "--> daemon-api-drift: cargo xtask gen-daemon-api"
(
  cd packages
  "$xtask_bin" gen-daemon-api
)

if git diff --exit-code -- docs/reference/daemon-api.md >/dev/null; then
  ok "daemon-api-drift: docs/reference/daemon-api.md matches generated wire tables"
else
  git --no-pager diff -- docs/reference/daemon-api.md | head -120 >&2 || true
  fail "daemon-api-drift: generated daemon API drift under docs/reference/daemon-api.md"
  exit 1
fi
