{ mkGuestEval, lib, flakeRoot, ... }:

let
  evaluated = mkGuestEval {
    modules = [
      (import (flakeRoot
        + "/packages/d2b-provider-network-local/nix/net.nix"))
    ];
  };
  dhcp = evaluated.config.systemd.network.networks."10-eth-dhcp";
in
{
  "network/guest-disables-catch-all-dhcp" = {
    expr = {
      match = dhcp.matchConfig.MACAddress;
    };
    expected = {
      match = "00:00:00:00:00:00";
    };
  };

  "network/guest-module-has-no-legacy-hierarchy-inputs" = {
    expr =
      let source = builtins.readFile
        (flakeRoot + "/packages/d2b-provider-network-local/nix/net.nix");
      in lib.all
        (needle: !(lib.hasInfix needle source))
        [ "cfg.envs" "d2b.vms" "d2b.realms" "processes.json" ];
    expected = true;
  };
}
