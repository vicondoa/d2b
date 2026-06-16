#!/usr/bin/env bash
# Auth-core eval smoke guard.
#
# The guest-control-health readiness non-goal was retired in W15, which migrates
# framework readiness onto the authenticated guest-control Health probe
# (ReadinessPredicate::GuestControlHealth). The `nixling exec` CLI non-goal was
# retired in W16, which intentionally landed the admin-only `vm exec`
# guest-control surface. This guard now just smoke-checks that the
# auth-core token-share / LoadCredential eval still evaluates.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

if ! command -v rg >/dev/null 2>&1; then
  fail "guest-control-auth-nongoals: rg is required"
fi

NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  nix eval --raw --impure --expr "import $ROOT/tests/guest-control-auth-eval.nix { }" >/dev/null

ok "guest-control-auth-nongoals: auth-core eval holds"
