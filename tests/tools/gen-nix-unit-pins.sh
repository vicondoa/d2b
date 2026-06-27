#!/usr/bin/env bash
# tests/tools/gen-nix-unit-pins.sh — regenerate the fail-closed nix-unit
# case-presence pins (tests/unit/nix/pinned/{common,<system>}.txt).
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

# Supported systems, derived from the flake's own `checks` (forAllSystems) —
# the same source of truth the gate uses — rather than hardcoded, so adding
# a system to flake.nix's `systems` automatically pins it. Sorted, so the
# first (the lexically-smallest, e.g. aarch64-linux) is the "base" whose case
# set defines common.txt; the others contribute a per-system delta. (This
# assumes the base's case set is a subset of every other system's — true
# while extra cases are x86-only graphics guards.)
mapfile -t SYSTEMS < <(
  nix eval --no-warn-dirty --raw "$ROOT#checks" \
    --apply 'cs: builtins.concatStringsSep "\n" (builtins.sort builtins.lessThan (builtins.attrNames cs))' \
    2>/dev/null
)
if [ "${#SYSTEMS[@]}" -eq 0 ]; then
  echo "gen-nix-unit-pins: could not derive systems from .#checks; falling back to defaults" >&2
  SYSTEMS=(aarch64-linux x86_64-linux)
fi
PIN_DIR="$ROOT/tests/unit/nix/pinned"
mkdir -p "$PIN_DIR"

case_names() {
  local sys="$1"
  nix eval --no-warn-dirty --raw --impure --expr '
    let
      f = builtins.getFlake "git+file://'"$ROOT"'";
      pkgs = f.inputs.nixpkgs.legacyPackages.'"$sys"';
      inputs = f.inputs // { self = f; };
      d2bModule = import '"$ROOT"'/nixos-modules { inherit inputs; };
      mkEval = modules: f.inputs.nixpkgs.lib.nixosSystem {
        system = "'"$sys"'"; modules = [ d2bModule ] ++ modules;
      };
      cases = import '"$ROOT"'/tests/unit/nix {
        lib = pkgs.lib; inherit pkgs; system = "'"$sys"'";
        flakeRoot = '"$ROOT"';
        d2bLib = import '"$ROOT"'/nixos-modules/lib.nix { lib = pkgs.lib; };
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
