{ lib, pkgs, flakeRoot, ... }:

let
  module = (import (flakeRoot + "/nixos-modules/host-daemon.nix")) {
    config.d2b = {
      site = {
        usePrebuiltHostTools = false;
        stateDir = "/var/lib/d2b";
        launcherUsers = [ "alice" ];
        adminUsers = [ "alice" ];
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
    };
    inherit lib pkgs;
  };
  socket = module.config.systemd.sockets.d2bd;
  service = module.config.systemd.services.d2bd;
in
{
  "daemon-activation/listener-provenance" = {
    expr = {
      path = socket.socketConfig.ListenSequentialPacket;
      owner = socket.socketConfig.SocketUser;
      group = socket.socketConfig.SocketGroup;
      mode = socket.socketConfig.SocketMode;
      fdName = socket.socketConfig.FileDescriptorName;
      service = socket.socketConfig.Service;
      accept = socket.socketConfig.Accept;
    };
    expected = {
      path = "/run/d2b/root.sock";
      owner = "root";
      group = "d2b";
      mode = "0660";
      fdName = "public.sock";
      service = "d2bd.service";
      accept = false;
    };
  };

  "daemon-activation/fd-only-service" = {
    expr =
      !(lib.hasInfix "--public-socket" service.serviceConfig.ExecStart)
      && builtins.elem "d2bd.socket" service.requires;
    expected = true;
  };

  "daemon-activation/starts-for-reconciliation" = {
    expr = service.wantedBy;
    expected = [ "multi-user.target" ];
  };
}
