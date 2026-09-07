{ lib, pkgs, system, nixpkgs, inputs, d2bModule, d2bLib, flakeRoot, modules }:

import ../helpers/surface.nix {
  inherit lib pkgs system nixpkgs inputs d2bModule d2bLib flakeRoot modules;
  name = "storage-volume";
  caseFiles = [
    {
      path = ../cases/volume-mounts.nix;
      names = [
        "volume-mounts/serial-null-defaults"
        "volume-mounts/v3-attachment-stays-declared-binding-input"
        "volume-mounts/v3-attachment-emits-no-durable-binding-resource"
        "volume-mounts/v3-attachment-emits-binding-worker-principal"
        "volume-mounts/v3-binding-worker-principal-follows-attachment"
      ];
    }
  ];
}
