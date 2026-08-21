# Nix authoring helpers for Provider/activation-nixos.
{ config, lib, ... }:

{
  options.d2b.providers.activationNixos = {
    retainedGenerations = lib.mkOption {
      type = lib.types.ints.between 1 16;
      default = 3;
      description = "Maximum generation records retained per execution target.";
    };
    providerRef = lib.mkOption {
      type = lib.types.str;
      default = "Provider/activation-nixos";
      readOnly = true;
    };
  };

  options.d2b._activationNixos = lib.mkOption {
    type = lib.types.attrsOf lib.types.anything;
    default = { };
    internal = true;
    visible = false;
  };

  config.d2b._activationNixos = {
    providerRef = config.d2b.providers.activationNixos.providerRef;
    retainedGenerations = config.d2b.providers.activationNixos.retainedGenerations;
    resourceType = "activation-nixos.d2bus.org.NixosGeneration";
    stateVolume = null;
    mkProviderResource = { config ? { } }: {
      type = "Provider";
      spec = {
        artifactId = "activation-nixos";
        inherit config;
      };
    };
    mkNixosGenerationResource = {
      name,
      executionRef,
      systemArtifactId,
      activationMode ? "switch",
      priorGenerationRef ? null
    }: {
      inherit name;
      type = "activation-nixos.d2bus.org.NixosGeneration";
      spec = {
        providerRef = "Provider/activation-nixos";
        inherit executionRef systemArtifactId;
        activationMode = activationMode;
      } // lib.optionalAttrs (priorGenerationRef != null) {
        inherit priorGenerationRef;
      };
    };
  };
}
