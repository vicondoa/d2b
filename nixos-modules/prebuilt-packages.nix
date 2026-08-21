{ pkgs, lib }:

let
  packages = import ../nix/prebuilt.nix { inherit pkgs lib; };
  selectPackage = name: fallback:
    let
      candidate =
        if packages != null && builtins.hasAttr name packages
        then builtins.getAttr name packages
        else null;
    in
      if candidate != null && (candidate.sourceBinary or null) == null
      then candidate
      else fallback;
in
(if packages == null then { } else packages) // { inherit selectPackage; }
