#!/usr/bin/env bash
# Runtime-shape tests for the guest-control token materializer.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

materializer="$ROOT/nixos-modules/guest-control-token-materialize.py"
scratch=$(nl_mktemp .guest-control-token.XXXXXX)

run_python() {
  if command -v python3 >/dev/null 2>&1; then
    python3 "$@"
  else
    nix shell --quiet --inputs-from "$ROOT" nixpkgs#python3 --command python3 "$@"
  fi
}

write_spec() {
  local spec=$1 source=$2 target=$3
  run_python - "$spec" "$source" "$target" <<'PY'
import json
import sys

spec, source, target = sys.argv[1:4]
with open(spec, "w", encoding="utf-8") as f:
    json.dump([{"name": "corp-vm", "source": source, "target": target}], f)
PY
}

expect_fail() {
  local label=$1 source=$2 kind=$3
  local spec="$scratch/$label.json"
  local target="$scratch/$label-target/token"
  local stderr="$scratch/$label.stderr"
  write_spec "$spec" "$source" "$target"
  if run_python "$materializer" "$spec" 2>"$stderr"; then
    fail "guest-control-token-materializer: $label unexpectedly succeeded"
  fi
  if ! grep -q "$kind" "$stderr"; then
    sed -n '1,20p' "$stderr" >&2
    fail "guest-control-token-materializer: $label did not report $kind"
  fi
  if grep -Fq "$source" "$stderr" || grep -Fq "$target" "$stderr"; then
    sed -n '1,20p' "$stderr" >&2
    fail "guest-control-token-materializer: $label leaked a token path"
  fi
}

expect_fail relative-source relative-token source-not-absolute
expect_fail store-root /nix/store source-in-nix-store
expect_fail store-child /nix/store/not-a-token source-in-nix-store
expect_fail missing-source /definitely-missing-nixling-token path-component-missing

ok "guest-control-token-materializer: fail-closed source validation is redacted"
