{ config, lib, ... }:

let
  artifactIdPattern = "[a-z][a-z0-9-]*";
  cfg = config.d2bNetworkLocalArtifacts;
in
{
  options.d2bNetworkLocalArtifacts = {
    providerArtifactId = lib.mkOption {
      type = lib.types.strMatching artifactIdPattern;
      default = "provider-network-local";
      description = "Provider package artifact ID.";
    };
    providerPackage = lib.mkOption {
      type = lib.types.package;
      description = "Network-local Provider package registered in the artifact catalog.";
    };
    netVmSystemArtifactId = lib.mkOption {
      type = lib.types.strMatching artifactIdPattern;
      description = "Required generic net-VM nixos-system artifact ID.";
    };
    netVmSystemPackage = lib.mkOption {
      type = lib.types.package;
      description = "Generic net-VM NixOS system registered in the artifact catalog.";
    };
  };

  config.d2b.artifacts = {
    "${cfg.providerArtifactId}" = {
      package = cfg.providerPackage;
      type = "provider";
    };
    "${cfg.netVmSystemArtifactId}" = {
      package = cfg.netVmSystemPackage;
      type = "nixos-system";
    };
  };
}
