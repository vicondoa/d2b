#!/usr/bin/env bash
# v1.1-P4 invariant gate: assert the deprecation warning text for
# `nixling.daemonExperimental.enable` is locked into
# `nixos-modules/assertions.nix` so operators see the same string
# in nixos-rebuild output AND the migration guide.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

assertions_module="$ROOT/nixos-modules/assertions.nix"

expected='nixling.daemonExperimental.enable is obsolete in v1.1; remove this option from your consumer flake because the broker socket/service are enabled by default. Leaving it set has no effect.'

if [ ! -f "$assertions_module" ]; then
  printf 'daemon-experimental-warning-eval: FAIL — %s missing\n' "$assertions_module" >&2
  exit 1
fi

if ! grep -F -- "$expected" "$assertions_module" >/dev/null 2>&1; then
  printf 'daemon-experimental-warning-eval: FAIL — warning text not found in %s\n' "$assertions_module" >&2
  printf '  expected literal string: %s\n' "$expected" >&2
  exit 1
fi

# Also verify it is a `warnings` entry (NOT an assertion) so
# leaving the option set does not block eval.
if ! grep -E '^\s*warnings\s*=' "$assertions_module" >/dev/null 2>&1; then
  printf 'daemon-experimental-warning-eval: FAIL — `warnings =` definition not found in %s\n' "$assertions_module" >&2
  exit 1
fi

printf 'daemon-experimental-warning-eval: PASS\n'
