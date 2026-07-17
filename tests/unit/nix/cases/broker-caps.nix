{ lib, pkgs, flakeRoot, ... }:

let
  module =
    (import (flakeRoot + "/nixos-modules/host-broker.nix") { inputs = { }; })
      {
        config.d2b = {
          site = {
            usePrebuiltHostTools = false;
            stateDir = "/var/lib/d2b";
            audit.retentionDays = 14;
            bundle.currentManifest = "/etc/d2b/bundle.json";
          };
          _realmPrincipals.localRoot = {
            controller = "d2bd";
            broker = "root";
            socketPrincipals.broker = {
              owner = "root";
              group = "d2bd";
              mode = "0660";
            };
          };
        };
        inherit lib pkgs;
      };
  service = module.config.systemd.services.d2b-priv-broker;
in
{
  "broker-caps/exact-local-root-set" = {
    expr =
      lib.sort builtins.lessThan service.serviceConfig.CapabilityBoundingSet;
    expected = [
      "CAP_CHOWN"
      "CAP_DAC_OVERRIDE"
      "CAP_DAC_READ_SEARCH"
      "CAP_FOWNER"
      "CAP_FSETID"
      "CAP_IPC_LOCK"
      "CAP_KILL"
      "CAP_LEASE"
      "CAP_MKNOD"
      "CAP_NET_ADMIN"
      "CAP_NET_RAW"
      "CAP_SETFCAP"
      "CAP_SETGID"
      "CAP_SETPCAP"
      "CAP_SETUID"
      "CAP_SYS_ADMIN"
      "CAP_SYS_RESOURCE"
    ];
  };

  "broker-caps/no-ambient-capabilities" = {
    expr = service.serviceConfig.AmbientCapabilities;
    expected = [ "" ];
  };
}
