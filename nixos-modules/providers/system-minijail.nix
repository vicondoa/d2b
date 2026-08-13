# Nix authoring facts for the fixed Provider/system-minijail boundary.
{ config, lib, ... }:

{
  options.d2b._systemMinijail = lib.mkOption {
    type = lib.types.attrsOf lib.types.anything;
    default = { };
    internal = true;
    visible = false;
  };

  config.d2b._systemMinijail = {
    providerRef = "Provider/system-minijail";
    resourceTypes = [ "Process" "EphemeralProcess" ];
    minimumKernel = "5.14";
    declaresStateVolume = false;
    persistentRootUnit = null;
  };
}
