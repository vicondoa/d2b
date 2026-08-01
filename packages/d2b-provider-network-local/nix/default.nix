{ lib, ... }:

{
  imports = [ ./artifacts.nix ];

  networking.networkmanager.unmanaged = lib.mkAfter [ "interface-name:d2b-*" ];
}
