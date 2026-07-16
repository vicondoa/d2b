{ lib, pkgs, flakeRoot, ... }:

let
  module =
    (import (flakeRoot + "/nixos-modules/host-broker.nix") { inputs = { }; })
      {
        config.d2b.site = {
          usePrebuiltHostTools = false;
          stateDir = "/var/lib/d2b";
          audit.retentionDays = 14;
          bundle.currentManifest = "/etc/d2b/bundle.json";
        };
        inherit lib pkgs;
      };
  socket = module.config.systemd.sockets.d2b-priv-broker;
  service = module.config.systemd.services.d2b-priv-broker;
in
{
  "broker-socket-activation/listener-provenance" = {
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
      path = "/run/d2b/broker.sock";
      owner = "root";
      group = "d2bd";
      mode = "0660";
      fdName = "priv.sock";
      service = "d2b-priv-broker.service";
      accept = false;
    };
  };

  "broker-socket-activation/fd-only-service" = {
    expr =
      !(lib.hasInfix "--socket-path" service.serviceConfig.ExecStart)
      && builtins.elem "d2b-priv-broker.socket" service.requires;
    expected = true;
  };
}
