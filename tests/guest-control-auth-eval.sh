#!/usr/bin/env bash
# Guest-control auth token delivery eval invariants.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  nix eval --raw --impure --expr "import $ROOT/tests/guest-control-auth-eval.nix { }" >/dev/null

ok "guest-control-auth-eval: token share and LoadCredential invariants hold"

