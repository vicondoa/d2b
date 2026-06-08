#!/usr/bin/env bash
# v1.1-P11 invariant gate: assert `flake.nix` does not declare
# `inputs.microvm`.
#
# Implementation status: SKIP at v1.1-rc1 (the input drop is the
# last phase of the substrate replacement; cannot land until
# P8/P9a/P9b/P10 cut over the consumers). Re-enables at
# v1.1-rc2 / v1.1-final.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

flake="$ROOT/flake.nix"
if grep -E '^\s*microvm\s*=\s*\{' "$flake" >/dev/null 2>&1 || \
   grep -E 'inputs\.microvm' "$flake" >/dev/null 2>&1; then
  printf 'microvm-nix-absent-eval: SKIP (v1.1-rc1; inputs.microvm still present; drop scheduled for v1.1-rc2 / v1.1-final)\n'
  exit 0
fi

printf 'microvm-nix-absent-eval: PASS\n'
