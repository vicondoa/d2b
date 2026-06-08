#!/usr/bin/env bash
# tests/ifname-collision.sh— canary.
#
# Covers:
#   - ifname-too-long (>= 16 bytes) — emitter refuses
#   - ifname-collision (hash dup)   — emitter + broker fail closed
#   - bundle-time mapping uniqueness across env/vm/role

set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

LOG=${TMPDIR:-/tmp}/nixling-ifname-collision.$$.log
: > "$LOG"
exec > >(tee -a "$LOG") 2>&1

cd "$ROOT/packages"

log "W3 s2 :: ifname-collision canary"

log " - ifname-too-long refused"
CARGO_BUILD_RUSTC_WRAPPER="" cargo test -p nixling-host --all-features --quiet -- \
  ifname_too_long_rejected ifname_invalid_character_rejected

log " - hash derivation is deterministic + role-disjoint"
CARGO_BUILD_RUSTC_WRAPPER="" cargo test -p nixling-host --all-features --quiet -- \
  derivation_is_deterministic role_distinguishes_bridge_vs_tap vm_changes_derivation

log " - emitter-time collision detection (bridge dup + bridge-vs-tap)"
CARGO_BUILD_RUSTC_WRAPPER="" cargo test -p nixling-host --all-features --quiet -- \
  detect_collisions_flags_duplicate_bridge detect_collisions_flags_bridge_vs_tap

log " - unique-set passes"
CARGO_BUILD_RUSTC_WRAPPER="" cargo test -p nixling-host --all-features --quiet -- \
  detect_collisions_passes_unique_set

log "OK: ifname-collision canary"
