#!/usr/bin/env bash
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}
scratch=$(mktemp -d "${TMPDIR:-/tmp}/nixling-legacy-group-denylist-self-test.XXXXXX")
denylist_script="$ROOT/tests/legacy-group-name-denylist.sh"
cleanup() { rm -rf -- "$scratch"; }
trap cleanup EXIT
rm -rf -- "$scratch"
mkdir -p "$scratch/nixos-modules" "$scratch/packages" "$scratch/tests" "$scratch/docs"

cat > "$scratch/nixos-modules/host-activation.nix" <<'CASE'
  for legacy_name in nixling-launcher nixling-launchers; do
CASE
cat > "$scratch/packages/forbidden.rs" <<'CASE'
const BAD: &str = "nixling-launcher";
CASE
cat > "$scratch/docs/out-of-scope.md" <<'CASE'
nixling-launchers is ignored here because docs are outside this gate.
CASE

if ROOT="$scratch" bash "$denylist_script" >/dev/null 2>&1; then
  echo 'legacy-group-name-denylist-self-test: FAIL — forbidden source line was not rejected' >&2
  exit 1
fi
rm -f "$scratch/packages/forbidden.rs"
ROOT="$scratch" bash "$denylist_script" >/dev/null
printf 'legacy-group-name-denylist-self-test: PASS\n'
