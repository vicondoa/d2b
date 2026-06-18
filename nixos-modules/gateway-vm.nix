# Auto-declare realm gateway guests from nixling.gateways.
{ config, lib, ... }:

let
  cfg = config.nixling;
  gateways = lib.filterAttrs (_: gw: gw.enable) cfg.gateways;

  gatewayVm = name: gw: {
    name = gw.vmName;
    value = {
      enable = true;
      autostart = false;
      env = gw.env;
      index = gw.index;
      ssh.user = lib.mkDefault "gateway";
      config = { pkgs, ... }: {
        networking.hostName = lib.mkDefault gw.vmName;
        users.users.gateway = {
          isNormalUser = true;
          extraGroups = [ "wheel" ];
        };
        environment.etc."nixling/gateway.json".text = builtins.toJSON {
          gateway = name;
          realm = gw.realm;
          stateDir = gw.stateDir;
          credentialPath = gw.credentialPath;
          relay = {
            inherit (gw.relay) namespace entity;
          };
          aca = {
            inherit (gw.aca) endpoint;
          };
          display = {
            inherit (gw.display) vsockPort waypipeCompression;
          };
        };
        environment.systemPackages = with pkgs; [
          waypipe
        ];
      };
    };
  };
in
{
  nixling.vms = lib.mkMerge [
    (lib.listToAttrs (lib.mapAttrsToList gatewayVm gateways))
  ];
}
