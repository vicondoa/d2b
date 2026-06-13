#!/usr/bin/env bash
# v1.1 invariant gate: assert the `nixling.daemonExperimental.enable`
# compatibility gate remains documented as a default-true,
# leave-at-default option, with operator migration guidance.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

options_module="$ROOT/nixos-modules/options-daemon.nix"
migration_guide="$ROOT/docs/how-to/migrate-nixling-v1-0-to-v1-1.md"

expected_option='consumers should leave it at its default'
expected_guide='Remove `nixling.daemonExperimental.enable`'

if [ ! -f "$options_module" ]; then
  printf 'daemon-experimental-warning-eval: FAIL — %s missing\n' "$options_module" >&2
  exit 1
fi

if [ ! -f "$migration_guide" ]; then
  printf 'daemon-experimental-warning-eval: FAIL — %s missing\n' "$migration_guide" >&2
  exit 1
fi

if ! grep -F -- "$expected_option" "$options_module" >/dev/null 2>&1; then
  printf 'daemon-experimental-warning-eval: FAIL — obsolete option text not found in %s\n' "$options_module" >&2
  printf '  expected literal string: %s\n' "$expected_option" >&2
  exit 1
fi

if ! grep -F -- "$expected_guide" "$migration_guide" >/dev/null 2>&1; then
  printf 'daemon-experimental-warning-eval: FAIL — migration instruction not found in %s\n' "$migration_guide" >&2
  printf '  expected literal string: %s\n' "$expected_guide" >&2
  exit 1
fi

printf 'daemon-experimental-warning-eval: PASS\n'
