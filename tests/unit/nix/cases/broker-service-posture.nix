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
  cfg = service.serviceConfig;
in
{
  "broker-service-posture/local-root-identity" = {
    expr = {
      user = cfg.User;
      group = cfg.Group;
      type = cfg.Type;
      notifyAccess = cfg.NotifyAccess;
    };
    expected = {
      user = "root";
      group = "root";
      type = "notify";
      notifyAccess = "main";
    };
  };

  "broker-service-posture/continuation" = {
    expr = {
      killMode = cfg.KillMode;
      slice = cfg.Slice;
      delegate = cfg.Delegate;
      restart = cfg.Restart;
    };
    expected = {
      killMode = "process";
      slice = "d2b.slice";
      delegate = true;
      restart = "on-failure";
    };
  };

  "broker-service-posture/no-global-manager-environment-mutation" = {
    expr =
      !(lib.hasInfix "set-environment" cfg.ExecStartPre)
      && cfg.EnvironmentFile == "-/run/d2b/broker/priv-broker.env";
    expected = true;
  };
}
