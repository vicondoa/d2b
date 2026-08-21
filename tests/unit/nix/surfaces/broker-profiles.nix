{ lib, pkgs, system, nixpkgs, inputs, d2bModule, d2bLib, flakeRoot, modules }:

import ../helpers/surface.nix {
  inherit lib pkgs system nixpkgs inputs d2bModule d2bLib flakeRoot modules;
  name = "broker-profiles";
  caseFiles = [
    ../cases/guest-broker.nix
    ../cases/host-tools-source.nix
    ../cases/prebuilt-broker.nix
  ];
}
