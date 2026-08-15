# Nix authoring helpers for Provider/system-systemd.
{ config, lib, ... }:

{
  options.d2b.providers.systemSystemd = {
    launchTimeoutSec = lib.mkOption {
      type = lib.types.ints.between 1 3600;
      default = 30;
    };
    terminationGraceSec = lib.mkOption {
      type = lib.types.ints.between 0 3600;
      default = 30;
    };
    userManagerCheckTimeout = lib.mkOption {
      type = lib.types.ints.between 1 60;
      default = 5;
    };
    maxConcurrentLaunches = lib.mkOption {
      type = lib.types.ints.between 1 256;
      default = 64;
    };
  };

  options.d2b._systemSystemd = lib.mkOption {
    type = lib.types.attrsOf lib.types.anything;
    default = { };
    internal = true;
    visible = false;
  };

  config.d2b._systemSystemd = {
    providerRef = "Provider/system-systemd";
    resourceTypes = [ "Process" "EphemeralProcess" ];
    config = {
      launchTimeoutSec = config.d2b.providers.systemSystemd.launchTimeoutSec;
      terminationGraceSec = config.d2b.providers.systemSystemd.terminationGraceSec;
      userManagerCheckTimeout = config.d2b.providers.systemSystemd.userManagerCheckTimeout;
      maxConcurrentLaunches = config.d2b.providers.systemSystemd.maxConcurrentLaunches;
    };
    persistentRootUnit = null;
  };
}
