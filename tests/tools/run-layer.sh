#!/usr/bin/env bash
# tests/tools/run-layer.sh <make_target> — run all not-yet-ported legacy
# scripts assigned to a `make` target in tests/migration-ledger.toml.
#
# W0: targets delegate to the legacy bash scripts (grouped by the ledger).
# W1+ each layer's implementation is repointed (nextest / nix-unit /
# runNixOSTest); a row flips status=ported and its legacy script is retired,
# so this runner naturally shrinks to nothing as the migration completes.
#
# Usage: run-layer.sh test-rust|test-drift|test-contract|test-nix-unit|
#                      test-flake|test-policy|test-integration|test-hardware|perf

set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}
export ROOT
LEDGER="$ROOT/tests/migration-ledger.toml"
target="${1:?usage: run-layer.sh <make_target>}"

[ -f "$LEDGER" ] || { echo "run-layer: missing ledger $LEDGER (run gen-migration-ledger.sh)" >&2; exit 1; }

# Extract `name`s whose make_target matches and status != ported.
mapfile -t scripts < <(awk -v t="$target" '
  /^\[\[script\]\]/ { name=""; mt=""; st="" }
  /^name = / { gsub(/^name = "|"$/,""); name=$0 }
  /^make_target = / { gsub(/^make_target = "|"$/,""); mt=$0 }
  /^status = / { sub(/#.*/,""); gsub(/^status = "| *"? *$|"/,""); st=$0 }
  /^$/ { if (name != "" && mt == t && st != "ported") print name; name="" }
  END { if (name != "" && mt == t && st != "ported") print name }
' "$LEDGER")

if [ "${#scripts[@]}" -eq 0 ]; then
  echo "run-layer[$target]: no legacy scripts remain (fully ported or empty group)"
  exit 0
fi

echo "run-layer[$target]: ${#scripts[@]} legacy script(s)"
rc=0
for s in "${scripts[@]}"; do
  [ -f "$ROOT/$s" ] || { echo "  MISSING $s (ledger drift — re-run check-inventory)" >&2; rc=1; continue; }
  printf '  -> %s\n' "$s"
  if ! bash "$ROOT/$s"; then
    echo "  FAIL $s" >&2; rc=1
  fi
done
exit "$rc"
