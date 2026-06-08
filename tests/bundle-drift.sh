#!/usr/bin/env bash
# nixling-core DTO -> committed JSON Schema drift.
#
# When cargo is not on PATH, re-execute through `nix shell` to acquire
# the toolchain (same bootstrap as tests/rust-workspace-checks.sh,
# resolved through the finding). The pinned rust-toolchain.toml
# under packages/ governs the version once cargo is reachable.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

cd "$ROOT"

if [ ! -f packages/nixling-core/src/bundle.rs ] \
  || [ ! -f packages/nixling-core/src/lib.rs ] \
  || ! grep -q 'pub mod bundle' packages/nixling-core/src/lib.rs \
  || [ ! -f packages/xtask/Cargo.toml ]; then
  log "no packages/nixling-core/src/bundle.rs — skipping bundle-drift (W1 unstaged)"
  exit 0
fi

if [ ! -d docs/reference/schemas ]; then
  fail "bundle-drift: docs/reference/schemas/ missing after W1 bundle DTOs landed"
fi

# Self-bootstrap toolchain through nix shell when cargo isn't on PATH.
# Matches the pattern in tests/rust-workspace-checks.sh.
if [ -z "${NIXLING_BUNDLE_DRIFT_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "bundle-drift: neither cargo nor nix is on PATH"
  fi
  log "  cargo not on PATH; re-entering via nix shell to acquire toolchain"
  export NIXLING_BUNDLE_DRIFT_IN_NIX_SHELL=1
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

log "--> bundle-drift: cargo xtask gen-schemas"
(
  cd packages
  "$xtask_bin" gen-schemas
)

if git diff --exit-code -- docs/reference/schemas/ >/dev/null; then
  ok "bundle-drift: generated schemas match committed docs/reference/schemas/"
else
  git --no-pager diff -- docs/reference/schemas/ | head -120 >&2 || true
  fail "bundle-drift: generated schema drift under docs/reference/schemas/"
fi
