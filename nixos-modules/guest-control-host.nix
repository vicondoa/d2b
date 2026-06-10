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
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion =
      vm.guest.control.auth.tokenFile == null
      || (
        lib.hasPrefix "/" vm.guest.control.auth.tokenFile
        && vm.guest.control.auth.tokenFile != "/nix/store"
        && !(lib.hasPrefix "/nix/store/" vm.guest.control.auth.tokenFile)
      );
    message = ''
      nixling.vms.${name}.guest.control.auth.tokenFile must be an absolute
      runtime path outside /nix/store. Do not use Nix path literals or
      flake-relative secret files; use a runtime secret path such as
      /run/secrets/nixling/${name}/guest-control-token.
    '';
  }) cfg.vms;

  system.activationScripts.nixlingGuestControlTokens =
    lib.stringAfter [ "users" ] ''
      ${pkgs.coreutils}/bin/printf '%s\n' ${lib.escapeShellArg (builtins.toJSON tokenSpecs)} \
        | ${pkgs.python3}/bin/python3 ${tokenMaterializer} -
    '';
}
