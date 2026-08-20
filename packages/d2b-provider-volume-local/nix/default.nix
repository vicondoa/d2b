{ lib, ... }:

{
  options.d2bVolumeLocal = {
    sourcePolicies = lib.mkOption {
      type = lib.types.listOf lib.types.attrs;
      default = [ ];
      description = "Opaque source-policy catalog for volume-local.";
    };
  };
}
