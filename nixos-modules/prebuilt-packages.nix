{ pkgs, lib }:

let
  packages = import ../nix/prebuilt.nix { inherit pkgs lib; };
in
if packages == null then { } else packages
