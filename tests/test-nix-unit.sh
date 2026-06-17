#!/usr/bin/env bash
# tests/test-nix-unit.sh — `make test-nix-unit`: build the nix-unit corpus check
# (flake.checks.<system>.nix-unit) for the native system.
#
# This is a FOCUSED convenience target for iterating on the declarative
# value/throw corpus under tests/unit/nix/. It is NOT part of `make test-unit`,
# because the root `nix flake check` run by `make test-flake` already evaluates
# the nix-unit check — running both would double the work.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
NL_LOG=${NL_LOG:-/dev/null}
export ROOT NL_LOG

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"
cd "$ROOT"

system=$(nix eval --raw --impure --expr builtins.currentSystem)
log "--> nix build .#checks.$system.nix-unit"
if nix build --no-link --print-out-paths ".#checks.$system.nix-unit"; then
  ok "nix-unit corpus ($system)"
else
  fail "nix-unit corpus ($system)"
  exit 1
fi

log "test-nix-unit OK"
