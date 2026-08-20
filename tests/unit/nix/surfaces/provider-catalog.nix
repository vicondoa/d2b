{ lib, pkgs, system, nixpkgs, inputs, d2bModule, d2bLib, flakeRoot, modules }:

import ../helpers/surface.nix {
  inherit lib pkgs system nixpkgs inputs d2bModule d2bLib flakeRoot modules;
  name = "provider-catalog";
  caseFiles = [{
    path = ../cases/provider-elf-shim.nix;
    names = [ "provider-elf-shim/positive-constructor" ];
  }];
}
