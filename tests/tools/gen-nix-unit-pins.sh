#!/usr/bin/env bash
# tests/tools/gen-nix-unit-pins.sh — regenerate the fail-closed nix-unit
# case-presence pins (tests/nix-unit/pinned/{common,<system>}.txt).
#
# `common.txt`        = case names present on EVERY system.
# `<system>.txt`      = extra case names present only on that system
#                       (e.g. x86_64-linux graphics cases guarded out of
#                       aarch64 via `lib.optionalAttrs (system == ...)`).
#
# The flake.checks.<sys>.nix-unit gate fails closed if any pinned case name
# is absent from the evaluated corpus, so a retired bash gate's nix-unit
# successor can't silently vanish. Run this after adding/removing cases.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}
export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"

# Systems the corpus is pinned for. The first is treated as the "base"
# whose case set defines common.txt; others contribute a per-system delta.
SYSTEMS=(aarch64-linux x86_64-linux)
PIN_DIR="$ROOT/tests/nix-unit/pinned"
mkdir -p "$PIN_DIR"

case_names() {
  local sys="$1"
  nix eval --no-warn-dirty --raw --impure --expr '
    let
      f = builtins.getFlake "git+file://'"$ROOT"'";
      pkgs = f.inputs.nixpkgs.legacyPackages.'"$sys"';
      inputs = f.inputs // { self = f; };
      nixlingModule = import '"$ROOT"'/nixos-modules { inherit inputs; };
      mkEval = modules: f.inputs.nixpkgs.lib.nixosSystem {
        system = "'"$sys"'"; modules = [ nixlingModule ] ++ modules;
      };
      cases = import '"$ROOT"'/tests/nix-unit {
        lib = pkgs.lib; inherit pkgs; system = "'"$sys"'";
        flakeRoot = '"$ROOT"';
        nl = import '"$ROOT"'/nixos-modules/lib.nix { lib = pkgs.lib; };
        inherit mkEval;
      };
    in pkgs.lib.concatStringsSep "\n" (builtins.attrNames cases)
  '
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

for sys in "${SYSTEMS[@]}"; do
  echo "evaluating $sys case names..." >&2
  case_names "$sys" | sort > "$tmp/$sys.names"
done

base="${SYSTEMS[0]}"
{
  echo "# nix-unit case-presence pins (common — present on every system)."
  echo "# Regenerate with: make nix-unit-pin"
  cat "$tmp/$base.names"
} > "$PIN_DIR/common.txt"
echo "wrote $PIN_DIR/common.txt ($(grep -cv '^#' "$PIN_DIR/common.txt") cases)" >&2

# Every supported system commits a per-system pin file (REQUIRED TO EXIST by
# the gate so deleting it fails closed); it may be header-only for a system
# with no system-specific cases (e.g. the base system's delta is empty).
for sys in "${SYSTEMS[@]}"; do
  delta=$(comm -23 "$tmp/$sys.names" "$tmp/$base.names")
  {
    echo "# nix-unit case-presence pins ($sys-only, e.g. graphics cases)."
    echo "# Header-only is valid: this system has no system-specific cases."
    echo "# Regenerate with: make nix-unit-pin"
    [ -n "$delta" ] && printf '%s\n' "$delta"
  } > "$PIN_DIR/$sys.txt"
  echo "wrote $PIN_DIR/$sys.txt ($(grep -cv '^#' "$PIN_DIR/$sys.txt") cases)" >&2
done
