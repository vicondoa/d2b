{ lib, pkgs, flakeRoot, ... }:

let
  identity = import (flakeRoot + "/nixos-modules/v2-identity.nix");
  principalModule =
    (import (flakeRoot + "/nixos-modules/realm-users.nix")) {
      config.d2b = {
        _index.realms.enabledList = [
          {
            placement = "host-local";
            realmName = "work";
            realmPath = "work.local-root";
            realmId = identity.deriveRealmId "work.local-root";
          }
        ];
        realms.work = {
          allowedUsers = [ "alice" ];
          allowedGroups = [ ];
        };
      };
      inherit lib;
    };
  realmPrincipals = principalModule.config.d2b._realmPrincipals;
  d2b = {
    site = {
      usePrebuiltHostTools = false;
      stateDir = "/var/lib/d2b";
      launcherUsers = [ "alice" ];
      adminUsers = [ "alice" ];
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
    _realmPrincipals = realmPrincipals;
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
  sockets =
    daemon.config.systemd.sockets // broker.config.systemd.sockets;
  endpointUnits =
    (map (name: "${name}.service") (builtins.attrNames services))
    ++ (map (name: "${name}.socket") (builtins.attrNames sockets));
in
{
  "local-root-endpoints/fixed-unit-set" = {
    expr = lib.sort builtins.lessThan endpointUnits;
    expected = [
      "d2b-priv-broker.service"
      "d2b-priv-broker.socket"
      "d2bd.service"
      "d2bd.socket"
    ];
  };

  "local-root-endpoints/fixed-unit-count" = {
    expr = builtins.length endpointUnits;
    expected = 4;
  };

  "local-root-endpoints/no-scalable-units" = {
    expr =
      builtins.length realmPrincipals.children == 1
      && lib.all
        (name:
          builtins.match ".*@.*" name == null
          && builtins.match "d2bd-r-.*" name == null
          && builtins.match "d2bbr-r-.*" name == null
          && builtins.match "d2b-.*-(gpu|snd|swtpm|video)\\.service" name == null)
        endpointUnits;
    expected = true;
  };

  "local-root-endpoints/actual-principal-contract" = {
    expr = {
      controllerUser = services.d2bd.serviceConfig.User;
      publicOwner = sockets.d2bd.socketConfig.SocketUser;
      publicGroup = sockets.d2bd.socketConfig.SocketGroup;
      brokerUser = services.d2b-priv-broker.serviceConfig.User;
      brokerGroup = services.d2b-priv-broker.serviceConfig.Group;
      brokerOwner = sockets.d2b-priv-broker.socketConfig.SocketUser;
      brokerSocketGroup = sockets.d2b-priv-broker.socketConfig.SocketGroup;
    };
    expected = {
      controllerUser = realmPrincipals.localRoot.controller;
      publicOwner = realmPrincipals.localRoot.socketPrincipals.public.owner;
      publicGroup = realmPrincipals.localRoot.socketPrincipals.public.group;
      brokerUser = realmPrincipals.localRoot.broker;
      brokerGroup = realmPrincipals.localRoot.broker;
      brokerOwner = realmPrincipals.localRoot.socketPrincipals.broker.owner;
      brokerSocketGroup =
        realmPrincipals.localRoot.socketPrincipals.broker.group;
    };
  };
}
