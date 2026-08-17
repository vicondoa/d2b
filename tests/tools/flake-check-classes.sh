#!/usr/bin/env bash
# tests/tools/flake-check-classes.sh - the single definition of how a flake
# check name is classified for the CI fan-out. Sourced, never executed.
#
# Three classes partition `checks.<native>.*`:
#
#   realized  - the shard must BUILD the check, not just instantiate it. These
#               are minutes-long, so CI runs them in their own unbounded job
#               rather than letting them sit behind a bounded matrix.
#   nix-unit  - already realized by the dedicated Nix-unit lane, which runs
#               `nix build` on exactly these names. Instantiating them a second
#               time in the flake-eval matrix is strictly redundant work.
#   eval      - everything else: instantiate-only, seconds each.
#
# The classification lives here so `tests/test-flake.sh` (which decides whether
# a shard builds or instantiates) and `tests/tools/flake-check-partition.sh`
# (which decides which job a shard is dispatched to) cannot disagree. A shard
# dispatched to the realized job but instantiated by the driver would report a
# green realized check that never built anything.

# Checks whose shard must realize the derivation. Keep this list minimal: each
# entry costs a full build on every PR.
D2B_FLAKE_REALIZED_CHECKS="video-binary-contract"

d2b_flake_check_is_realized() {
  local candidate=$1 name
  for name in $D2B_FLAKE_REALIZED_CHECKS; do
    [ "$candidate" = "$name" ] && return 0
  done
  return 1
}

# Mirrors the Nix-side predicate in tests/test-nix-unit.sh: the corpus check
# itself plus every `nix-unit-<shard>`.
d2b_flake_check_is_nix_unit() {
  case "$1" in
    nix-unit | nix-unit-*) return 0 ;;
    *) return 1 ;;
  esac
}
