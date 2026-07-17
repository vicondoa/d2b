{ lib, pkgs, flakeRoot, ... }:

let
  d2b = {
    site = {
      usePrebuiltHostTools = false;
      stateDir = "/var/lib/d2b";
      launcherUsers = [ ];
      adminUsers = [ ];
      audit.retentionDays = 14;
      bundle.currentManifest = "/etc/d2b/bundle.json";
    };
    daemon = {
      autostart.parallelism = 3;
      lifecycle = {
        gracefulShutdown.timeoutSeconds = 90;
        liveActivation.timeoutSeconds = 90;
      };
    };
    _bundle = {
      bundle.path = "/nix/store/d2b-bundle";
      providerRegistryV2Json.path = "/nix/store/d2b-provider-registry";
    };
    _realmPrincipals.localRoot = {
      controller = "d2bd";
      broker = "root";
      publicGroup = "d2b";
      socketPrincipals = {
        public = {
          owner = "d2bd";
          group = "d2b";
          mode = "0660";
        };
        broker = {
          owner = "root";
          group = "d2bd";
          mode = "0660";
        };
      };
    };
  };
  args = {
    config = { inherit d2b; };
    inherit lib pkgs;
  };
  daemon =
    (import (flakeRoot + "/nixos-modules/host-daemon.nix")) args;
  broker =
    (import (flakeRoot + "/nixos-modules/host-broker.nix") { inputs = { }; })
      args;
  services =
    daemon.config.systemd.services // broker.config.systemd.services;
  forbiddenPatterns = [
    ".*@.*"
    "d2bd-r-.*"
    "d2bbr-r-.*"
    "d2b-.*-(gpu|snd|swtpm|video)"
  ];
in
{
  "restart-policy/fixed-services-only" = {
    expr = lib.sort builtins.lessThan (builtins.attrNames services);
    expected = [ "d2b-priv-broker" "d2bd" ];
  };

  "restart-policy/no-child-or-workload-service" = {
    expr = lib.all
      (name:
        lib.all (pattern: builtins.match pattern name == null)
          forbiddenPatterns)
      (builtins.attrNames services);
    expected = true;
  };

  "restart-policy/controller-continuation" = {
    expr = services.d2bd.serviceConfig.KillMode;
    expected = "process";
  };

  "restart-policy/broker-continuation" = {
    expr = services.d2b-priv-broker.serviceConfig.KillMode;
    expected = "process";
  };
}
