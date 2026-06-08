#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

log "==> tests/manifest-fuzz-bounded.sh"
if ! grep -Eq '^fuzz[[:space:]]*=' "$ROOT/packages/nixling-core/Cargo.toml" \
  || ! grep -Rqs 'bolero' "$ROOT/packages/nixling-core"; then
  log "  SKIP: nixling-core bolero harness has not landed in this worktree yet"
  exit 0
fi

if [ -z "${NIXLING_MANIFEST_FUZZ_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "manifest-fuzz-bounded: neither cargo nor nix is on PATH"
    exit 1
  fi
  export NIXLING_MANIFEST_FUZZ_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

(cd "$ROOT/packages" && CARGO_TARGET_DIR="$(nl_cargo_target_dir workspace)" cargo test --release -p nixling-core --features fuzz -- --runs 10000)
ok "bounded manifest fuzz harness completed 10000 runs"
