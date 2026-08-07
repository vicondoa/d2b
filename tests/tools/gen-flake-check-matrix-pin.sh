#!/usr/bin/env bash
# tests/tools/gen-flake-check-matrix-pin.sh - regenerate / verify the committed
# pins of both native-system flake check inventories.
#
# The `pr-l1-static-fast` workflow discovers its hosted-runner native matrix
# via `make test-flake-list`. That list may intentionally filter checks that are
# too large or unstable for GitHub-hosted runners (for example
# `fixture-smoke-full`). This pin tracks the full static
# native `flake.checks.<system>.*` sets instead: adding/removing a flake check
# fails the drift gate until `make flake-matrix-pin` is run, forcing a reviewer
# to confirm whether the new check is hosted-runner-sharded, local/manual only,
# or otherwise covered.
#
#   make flake-matrix-pin                              # regenerate the pin
#   bash tests/tools/gen-flake-check-matrix-pin.sh --check   # diff (CI gate)
#
# The public command regenerates both native inventories. Set
# D2B_FLAKE_MATRIX_SYSTEM only for a single-system diagnostic run. This is
# CI-matrix plumbing, not a test case; it lives in tests/tools/ and is invoked
# by tests/unit/gates/flake-check-matrix-sync.sh (run by `make test-drift`).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}

if [ -n "${D2B_FLAKE_MATRIX_SYSTEM:-}" ]; then
  case "$D2B_FLAKE_MATRIX_SYSTEM" in
    x86_64-linux|aarch64-linux)
      systems=("$D2B_FLAKE_MATRIX_SYSTEM")
      ;;
    *)
      echo "flake-check-matrix: unsupported native system '$D2B_FLAKE_MATRIX_SYSTEM'" >&2
      exit 2
      ;;
  esac
else
  systems=(x86_64-linux aarch64-linux)
fi

export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"
cd "$ROOT"

# git+file:// (never a bare path): mirror tests/lib.sh d2b_flake_ref so the
# sibling cargo target/ + scratch dirs stay invisible to the eval.
flake_ref="git+file://$ROOT"

mode="write"
if [ "${1:-}" = "--check" ]; then
  mode="check"
fi

render() {
  local system=$1
  local live=$2
  printf '# Flake-check pin: full names of flake.checks.%s.*.\n' "$system"
  printf '# The hosted-runner dynamic matrix may intentionally filter this set.\n'
  printf '# Regenerate with: make flake-matrix-pin\n'
  printf '%s\n' "$live"
}

rc=0
for system in "${systems[@]}"; do
  pin="$ROOT/tests/golden/flake-check-matrix/$system.txt"
  # attrNames + sort: the authoritative, deterministic full check set. This
  # may be a superset of the hosted-runner matrix from `make test-flake-list`.
  live=$(nix eval --raw "${flake_ref}#checks.${system}" --apply \
    'cs: builtins.concatStringsSep "\n" (builtins.sort (a: b: a < b) (builtins.attrNames cs))')

  if [ "$mode" = "check" ]; then
    if [ ! -f "$pin" ]; then
      echo "flake-check-matrix pin: MISSING $pin - run 'make flake-matrix-pin'" >&2
      rc=1
      continue
    fi
    tmp=$(mktemp)
    render "$system" "$live" > "$tmp"
    if diff -u "$pin" "$tmp"; then
      echo "flake-check-matrix pin: up to date ($(printf '%s\n' "$live" | grep -c .) checks for $system)"
    else
      {
        echo ""
        echo "FAIL: flake.checks.$system drifted from the committed CI-matrix pin."
        echo "A flake check was added or removed. Run 'make flake-matrix-pin',"
        echo "then confirm the new check is covered by the hosted matrix,"
        echo "a local/manual gate, or another explicit validation path."
      } >&2
      rc=1
    fi
    rm -f "$tmp"
  else
    mkdir -p "$(dirname "$pin")"
    render "$system" "$live" > "$pin"
    echo "wrote $pin ($(printf '%s\n' "$live" | grep -c .) checks for $system)" >&2
  fi
done

exit "$rc"
