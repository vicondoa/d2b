{ lib, pkgs, system, nixpkgs, inputs, d2bModule, d2bLib, flakeRoot, modules }:

import ../helpers/surface.nix {
  inherit lib pkgs system nixpkgs inputs d2bModule d2bLib flakeRoot modules;
  name = "storage-volume";
  caseFiles = [
    {
      path = ../cases/volume-mounts.nix;
      names = [ "volume-mounts/serial-null-defaults" ];
    }
  ];
}
