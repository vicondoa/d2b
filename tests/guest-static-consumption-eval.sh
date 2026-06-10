#!/usr/bin/env bash
# Prove guest VM evals consume nixling's static guest package outputs.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  nix eval --raw --impure --expr "import $ROOT/tests/guest-static-consumption-eval.nix { }" >/dev/null

ok "guest-static-consumption-eval: VM guest eval consumes static guest outputs"
