#!/usr/bin/env bash
# tests/test-flake-list.sh — `make test-flake-list`: print the native-system
# flake check names as a compact JSON array on stdout (all logs go to stderr).
#
# The CI dynamic matrix consumes this to fan `make test-flake D2B_FLAKE_CHECK=<n>`
# shards out across runners (see .github/workflows/pr-l1-static-fast.yml). Keep
# stdout PURE JSON so it can feed `$GITHUB_OUTPUT` + `fromJSON()` directly:
#
#   echo "checks=$(make -s test-flake-list)" >> "$GITHUB_OUTPUT"
#
# This is plumbing for the sharded `make test-flake`, not a test case itself, so
# it is listed in the ORCH exclude set of tests/tools/gen-migration-ledger.sh.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}

export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"
cd "$ROOT"

# git+file:// (never a bare path): mirror tests/lib.sh d2b_flake_ref so the
# sibling cargo target/ + scratch dirs stay invisible to the eval.
flake_ref="git+file://$ROOT"

native=$(nix eval --raw --impure --expr builtins.currentSystem 2>/dev/null || echo "native")
echo "test-flake-list: enumerating checks.$native.*" >&2

# attrNames forces only the keys of the checks attrset (cheap) — it does not
# evaluate each check's derivation. Keep every check in the dynamic matrix; if
# an evaluator segfaults, tests/test-flake.sh emits grouped diagnostics and a
# gdb backtrace for the failing evaluator command.
nix eval --json "${flake_ref}#checks.${native}" --apply builtins.attrNames
