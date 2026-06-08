#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
BASELINE=${BASELINE:-$ROOT/tests/golden/manifest_v04/baseline-vms.json}
ASSERT_SCRIPT=${ASSERT_SCRIPT:-$ROOT/tests/golden/manifest_v04/assert-roundtrip.sh}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

cd "$ROOT"

if [ -z "${NIXLING_MANIFEST_V04_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "manifest-v04-roundtrip: neither cargo nor nix is on PATH"
  fi
  log "  cargo not on PATH; re-entering via nix shell to acquire toolchain"
  export NIXLING_MANIFEST_V04_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

if [ ! -f "$BASELINE" ]; then
  fail "manifest-v04-roundtrip: baseline fixture $BASELINE is missing"
fi
if [ ! -x "$ASSERT_SCRIPT" ]; then
  fail "manifest-v04-roundtrip: assertion script $ASSERT_SCRIPT is missing or not executable"
fi

scratch=$(nl_mktemp .manifest-v04-roundtrip.XXXXXX)
rendered=$scratch/rendered-vms.json
add_cleanup "rm -rf -- \"$scratch\""

manifest_v04_check_bin=$(nl_cargo_bin_path workspace manifest_v04_check)
if [ ! -x "$manifest_v04_check_bin" ]; then
  (
    cd packages
    CARGO_TARGET_DIR="$(nl_cargo_target_dir workspace)" cargo build -q --manifest-path "$ROOT/packages/Cargo.toml" -p xtask --bin manifest_v04_check
  )
fi

log "--> manifest-v04-roundtrip: cargo run -p xtask --bin manifest_v04_check"
(
  cd packages
  "$manifest_v04_check_bin" "$BASELINE" "$rendered"
)

bash "$ASSERT_SCRIPT" "$BASELINE" "$rendered"
ok "manifest-v04-roundtrip: baseline parses and round-trips byte-identically"
