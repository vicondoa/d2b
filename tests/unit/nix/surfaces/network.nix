{ lib, pkgs, system, nixpkgs, inputs, d2bModule, d2bLib, flakeRoot, modules }:

import ../helpers/surface.nix {
  inherit lib pkgs system nixpkgs inputs d2bModule d2bLib flakeRoot modules;
  name = "network";
  caseFiles = [
    {
      path = ../cases/net-vm-network.nix;
      names = [
        "net-vm-network/eth-dhcp-match-type-not-ether"
        "net-vm-network/eth-dhcp-match-mac-sentinel"
      ];
    }
  ];
}
