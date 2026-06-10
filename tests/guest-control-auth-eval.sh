#!/usr/bin/env bash
# Guest-control auth token delivery eval invariants.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  nix eval --raw --impure --expr "import $ROOT/tests/guest-control-auth-eval.nix { }" >/dev/null

if NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  nix eval --raw --impure --expr "import $ROOT/tests/guest-control-auth-eval.nix { tokenFile = \"/nix/store\"; }" >/dev/null 2>&1; then
  fail "guest-control-auth-eval: /nix/store tokenFile unexpectedly passed"
fi

if NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  nix eval --raw --impure --expr "import $ROOT/tests/guest-control-auth-eval.nix { guestControlEnable = false; }" >/dev/null 2>&1; then
  fail "guest-control-auth-eval: tokenFile without guest.control.enable unexpectedly passed"
fi

ok "guest-control-auth-eval: token share and LoadCredential invariants hold"
