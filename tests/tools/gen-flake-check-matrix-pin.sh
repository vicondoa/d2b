#!/usr/bin/env bash
# tests/tools/gen-flake-check-matrix-pin.sh — regenerate / verify the committed
# pin of x86_64-linux flake check names that the CI dynamic matrix shards over.
#
# The `pr-l1-static-fast` workflow shards the x86_64 flake check one job per
# `flake.checks.x86_64-linux.*` entry (enumerated at CI time by
# `make test-flake-list`). That keeps the matrix in sync BY CONSTRUCTION, but a
# silently-added or -removed check would change CI coverage with no diff a human
# reviews. This pin makes the check SET explicit: adding/removing a flake check
# fails the drift gate until `make flake-matrix-pin` is run, forcing a reviewer
# to confirm the new shard coverage (and the `test-flake-x86` aggregator still
# gates it).
#
#   make flake-matrix-pin                              # regenerate the pin
#   bash tests/tools/gen-flake-check-matrix-pin.sh --check   # diff (CI gate)
#
# This is CI-matrix plumbing, not a test case; it lives in tests/tools/ and is
# invoked by tests/unit/gates/flake-check-matrix-sync.sh (run by `make test-drift`).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}

SYSTEM=${NL_FLAKE_MATRIX_SYSTEM:-x86_64-linux}
PIN="$ROOT/tests/golden/flake-check-matrix/$SYSTEM.txt"

export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"
cd "$ROOT"

# git+file:// (never a bare path): mirror tests/lib.sh nl_flake_ref so the
# sibling cargo target/ + scratch dirs stay invisible to the eval.
flake_ref="git+file://$ROOT"

mode="write"
if [ "${1:-}" = "--check" ]; then
  mode="check"
fi

# attrNames + sort: the authoritative, deterministic set of check names. This is
# the SAME enumeration `make test-flake-list` feeds the CI matrix.
live=$(nix eval --raw "${flake_ref}#checks.${SYSTEM}" --apply \
  'cs: builtins.concatStringsSep "\n" (builtins.sort (a: b: a < b) (builtins.attrNames cs))')

render() {
  printf '# CI dynamic-matrix pin: names of flake.checks.%s.* that the\n' "$SYSTEM"
  printf '# pr-l1-static-fast "flake-eval-x86" matrix shards over (one job each).\n'
  printf '# Regenerate with: make flake-matrix-pin\n'
  printf '%s\n' "$live"
}

if [ "$mode" = "check" ]; then
  if [ ! -f "$PIN" ]; then
    echo "flake-check-matrix pin: MISSING $PIN — run 'make flake-matrix-pin'" >&2
    exit 1
  fi
  tmp=$(mktemp)
  trap 'rm -f "$tmp"' EXIT
  render > "$tmp"
  if diff -u "$PIN" "$tmp"; then
    echo "flake-check-matrix pin: up to date ($(printf '%s\n' "$live" | grep -c .) checks for $SYSTEM)"
  else
    {
      echo ""
      echo "FAIL: flake.checks.$SYSTEM drifted from the committed CI-matrix pin."
      echo "A flake check was added or removed, so the sharded 'flake-eval-x86'"
      echo "matrix coverage changed. Run 'make flake-matrix-pin', then confirm the"
      echo "new check is shard-covered and the test-flake-x86 aggregator gates it."
    } >&2
    exit 1
  fi
else
  mkdir -p "$(dirname "$PIN")"
  render > "$PIN"
  echo "wrote $PIN ($(printf '%s\n' "$live" | grep -c .) checks for $SYSTEM)" >&2
fi
