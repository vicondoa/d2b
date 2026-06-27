#!/usr/bin/env bash
# tests/tools/gen-flake-check-matrix-pin.sh — regenerate / verify the committed
# pin of x86_64-linux flake check names.
#
# The `pr-l1-static-fast` workflow discovers its hosted-runner x86_64 matrix
# via `make test-flake-list`. That list may intentionally filter checks that are
# too large or unstable for GitHub-hosted runners (for example
# `fixture-smoke-full`). This pin tracks the full static
# `flake.checks.x86_64-linux.*` set instead: adding/removing a flake check fails
# the drift gate until `make flake-matrix-pin` is run, forcing a reviewer to
# confirm whether the new check is hosted-runner-sharded, local/manual only, or
# otherwise covered.
#
#   make flake-matrix-pin                              # regenerate the pin
#   bash tests/tools/gen-flake-check-matrix-pin.sh --check   # diff (CI gate)
#
# This is CI-matrix plumbing, not a test case; it lives in tests/tools/ and is
# invoked by tests/unit/gates/flake-check-matrix-sync.sh (run by `make test-drift`).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}

SYSTEM=${D2B_FLAKE_MATRIX_SYSTEM:-x86_64-linux}
PIN="$ROOT/tests/golden/flake-check-matrix/$SYSTEM.txt"

export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"
cd "$ROOT"

# git+file:// (never a bare path): mirror tests/lib.sh d2b_flake_ref so the
# sibling cargo target/ + scratch dirs stay invisible to the eval.
flake_ref="git+file://$ROOT"

mode="write"
if [ "${1:-}" = "--check" ]; then
  mode="check"
fi

# attrNames + sort: the authoritative, deterministic full check set. This may
# be a superset of the hosted-runner matrix emitted by `make test-flake-list`.
live=$(nix eval --raw "${flake_ref}#checks.${SYSTEM}" --apply \
  'cs: builtins.concatStringsSep "\n" (builtins.sort (a: b: a < b) (builtins.attrNames cs))')

render() {
  printf '# Flake-check pin: full names of flake.checks.%s.*.\n' "$SYSTEM"
  printf '# The hosted-runner dynamic matrix may intentionally filter this set.\n'
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
      echo "A flake check was added or removed. Run 'make flake-matrix-pin',"
      echo "then confirm the new check is covered by the hosted matrix,"
      echo "a local/manual gate, or another explicit validation path."
    } >&2
    exit 1
  fi
else
  mkdir -p "$(dirname "$PIN")"
  render > "$PIN"
  echo "wrote $PIN ($(printf '%s\n' "$live" | grep -c .) checks for $SYSTEM)" >&2
fi
