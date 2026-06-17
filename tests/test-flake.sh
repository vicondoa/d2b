#!/usr/bin/env bash
# tests/test-flake.sh — `make test-flake`: `nix flake check` for the build's
# NATIVE system only (bounded memory).
#
# CI runs this as a matrix across architectures (x86_64-linux on ubuntu-latest,
# aarch64-linux on ubuntu-24.04-arm), so each evaluator process holds a single
# system's checks. The previous monolithic `nix flake check --all-systems`
# cross-evaluated both architectures in one process and OOM-killed the 16 GB
# GitHub runner once the rearchitecture grew flake.checks (nix-unit corpus,
# cargo-deny/cargo-audit derivations, more example evals).
#
# Set NL_FLAKE_ALL_SYSTEMS=1 to cross-evaluate every supported system in one
# process (the heavier `make check` / tests/static.sh local gate does this on a
# large-memory host).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
NL_LOG=${NL_LOG:-/dev/null}
export ROOT NL_LOG

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"
cd "$ROOT"

# git+file:// (never a bare path): source-capture from the git tree only, so the
# sibling cargo target/ + scratch dirs stay invisible to the eval (disk-hygiene
# contract — see tests/lib.sh nl_flake_ref).
flake_ref=$(nl_flake_ref "$ROOT")

systems_flag=()
if [ "${NL_FLAKE_ALL_SYSTEMS:-0}" = 1 ]; then
  systems_flag=(--all-systems)
  log "--> nix flake check --no-build --all-systems"
else
  native=$(nix eval --raw --impure --expr builtins.currentSystem 2>/dev/null || echo "native")
  log "--> nix flake check --no-build (native system: $native)"
fi

if nix flake check "$flake_ref" --no-build "${systems_flag[@]}"; then
  ok "nix flake check"
else
  fail "nix flake check"
  exit 1
fi

log "test-flake OK"
