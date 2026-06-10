{ config, pkgs, lib, ... }:

let
  cfg = config.nixling;
  enabledGuestControlVms =
    lib.filterAttrs (_: vm: vm.enable && vm.guest.control.enable) cfg.vms;
  tokenSpecs = lib.mapAttrsToList (name: vm: {
    inherit name;
    source = vm.guest.control.auth.tokenFile;
    target = "${cfg.site.stateDir}/guest-control-${name}/token";
  }) enabledGuestControlVms;
  tokenSpecsFile = pkgs.writeText "nixling-guest-control-token-specs.json"
    (builtins.toJSON tokenSpecs);
  tokenMaterializer = ./guest-control-token-materialize.py;
in
{
  assertions = lib.mapAttrsToList (name: vm: {
    assertion = vm.guest.control.enable || vm.guest.control.auth.tokenFile == null;
    message = ''
      nixling.vms.${name}.guest.control.auth.tokenFile is set, but
      nixling.vms.${name}.guest.control.enable is false. Enable guest-control
      token delivery or remove the tokenFile setting.
    '';
  }) cfg.vms;

  system.activationScripts.nixlingGuestControlTokens =
    lib.stringAfter [ "users" ] ''
      ${pkgs.python3}/bin/python3 ${tokenMaterializer} ${tokenSpecsFile}
    '';
}
