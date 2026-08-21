{ lib, pkgs, system, nixpkgs, inputs, d2bModule, d2bLib, flakeRoot, modules }:

import ../helpers/surface.nix {
  inherit lib pkgs system nixpkgs inputs d2bModule d2bLib flakeRoot modules;
  name = "desktop-interaction";
  caseFiles = [ ];
}
