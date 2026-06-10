#!/usr/bin/env bash
# Build and validate static guest binaries for the current host system.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

system=$(nix eval --raw --impure --expr builtins.currentSystem)
NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  nix build "$ROOT#checks.$system.guest-static-elf" --no-link

ok "guest-static-elf: built static guest ELF check for $system"
