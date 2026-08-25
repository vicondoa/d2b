# Nix-unit coverage for a gateway's two authority domains.
{ mkGuestEval, lib, pkgs, flakeRoot, ... }:

let
  d2bd = pkgs.runCommand "d2bd-gateway-component-session-test" { } ''
    mkdir -p "$out/bin"
    touch "$out/bin/d2bd"
  '';
  broker = pkgs.runCommand "d2b-broker-gateway-component-session-test" { } ''
    mkdir -p "$out/bin"
    touch "$out/bin/d2b-broker"
  '';
  optionSinks = { lib, ... }: {
    options.environment.systemPackages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
    options.systemd.services = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
    options.systemd.sockets = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
    options.systemd.tmpfiles.rules = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
    };
    options.users.users = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
    options.users.groups = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
  };
  childZone = { ... }: {
    systemd.sockets.d2b-broker-child-zone = {
      socketConfig.ListenSequentialPacket = "/run/d2b/child-zone-broker.sock";
    };
    systemd.services.d2b-broker-child-zone = {
      serviceConfig.ExecStart =
        "${broker}/bin/d2b-broker host --authority-id child-zone";
    };
    systemd.services.d2bd-child-zone = {
      serviceConfig.ExecStart =
        "${d2bd}/bin/d2bd host --config /etc/d2b/child-zone-daemon.json";
    };
  };
  evaluated = (mkGuestEval {
    modules = [
      optionSinks
      (import (flakeRoot + "/nixos-modules/component-session.nix"))
      (import (flakeRoot + "/nixos-modules/guest-broker.nix"))
      childZone
      ({ ... }: {
        d2b.componentSession = {
          enable = true;
          guestConfigPath = null;
          shell = {
            enable = false;
            defaultName = "default";
            maxSessions = 8;
            maxAttached = 1;
          };
        };
      })
    ];
    specialArgs = {
      d2bInputs = { };
      d2bHostTools = { inherit d2bd broker; };
      d2bUsePrebuiltHostTools = false;
      name = "gateway";
    };
  }).config;
  guest = evaluated.systemd.services.d2bd-guest.serviceConfig;
  child = evaluated.systemd.services.d2bd-child-zone.serviceConfig;
  guestBroker = evaluated.systemd.services.d2b-broker-guest.serviceConfig;
  childBroker = evaluated.systemd.services.d2b-broker-child-zone.serviceConfig;
in
{
  "gateway-component-session/uses-separate-daemon-modes" = {
    expr = {
      guest = lib.hasInfix "/bin/d2bd guest " guest.ExecStart;
      child = lib.hasInfix "/bin/d2bd host " child.ExecStart;
      sameDaemonArtifact =
        builtins.head (lib.splitString " " guest.ExecStart)
        == builtins.head (lib.splitString " " child.ExecStart);
      guestPublic = lib.hasInfix "public.sock" guest.ExecStart;
      childPublic = lib.hasInfix "child-zone-daemon.json" child.ExecStart;
    };
    expected = {
      guest = true;
      child = true;
      sameDaemonArtifact = true;
      guestPublic = false;
      childPublic = true;
    };
  };

  "gateway-component-session/uses-separate-broker-profiles-and-sockets" = {
    expr = {
      guest = lib.hasInfix "/bin/d2b-broker guest " guestBroker.ExecStart;
      child = lib.hasInfix "/bin/d2b-broker host " childBroker.ExecStart;
      sameBrokerArtifact =
        builtins.head (lib.splitString " " guestBroker.ExecStart)
        == builtins.head (lib.splitString " " childBroker.ExecStart);
      guestSocket =
        evaluated.systemd.sockets.d2b-broker-guest.socketConfig.ListenSequentialPacket;
      childSocket =
        evaluated.systemd.sockets.d2b-broker-child-zone.socketConfig.ListenSequentialPacket;
    };
    expected = {
      guest = true;
      child = true;
      sameBrokerArtifact = true;
      guestSocket = "/run/d2b/guest-broker.sock";
      childSocket = "/run/d2b/child-zone-broker.sock";
    };
  };
}
