{ config, pkgs, lib, ... }:

let
  cfg = config.nixling;
  nl = import ./lib.nix { inherit lib; };
  enabledGuestControlVms =
    lib.filterAttrs (_: vm: vm.enable && vm.guest.control.enable) cfg.vms;
  usernamePattern = "^[a-z][a-z0-9_-]{0,31}$";
  unique = xs: lib.length xs == lib.length (lib.unique xs);
  usernameValid = user: builtins.match usernamePattern user != null;
  tokenSpecs = lib.mapAttrsToList (name: vm: {
    inherit name;
    source = vm.guest.control.auth.tokenFile;
    target = "${cfg.site.stateDir}/guest-control-${name}/token";
    readerGid = nl.stablePrincipalId "nixling-${name}-gctlfs";
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
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion =
      !vm.guest.exec.enable
      || vm.guest.control.enable;
    message = ''
      nixling.vms.${name}.guest.exec.enable requires
      nixling.vms.${name}.guest.control.enable because exec policy is enforced
      by the guest-control plane.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion =
      vm.guest.exec.enable
      || (!vm.guest.exec.allowRoot && vm.guest.exec.users == [ ]);
    message = ''
      nixling.vms.${name}.guest.exec.allowRoot/users are set, but
      nixling.vms.${name}.guest.exec.enable is false. Enable guest exec policy
      validation or remove the exec allowlist settings.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion =
      !vm.guest.exec.enable
      || vm.guest.exec.allowRoot
      || vm.guest.exec.users != [ ];
    message = ''
      nixling.vms.${name}.guest.exec.enable is true, but no exec target is
      allowed. Set guest.exec.users to at least one non-root guest user or set
      guest.exec.allowRoot = true.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion = unique vm.guest.exec.users;
    message = ''
      nixling.vms.${name}.guest.exec.users must not contain duplicate users.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion = lib.all usernameValid vm.guest.exec.users;
    message = ''
      nixling.vms.${name}.guest.exec.users entries must match
      ${usernamePattern}; wildcard, root-like, or path-like names are not
      accepted.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion = !(builtins.elem "root" vm.guest.exec.users);
    message = ''
      nixling.vms.${name}.guest.exec.users must not include root. Use
      nixling.vms.${name}.guest.exec.allowRoot for the separate root-exec
      policy gate.
    '';
  }) cfg.vms;

  system.activationScripts.nixlingGuestControlTokens =
    lib.stringAfter [ "users" ] ''
      ${pkgs.coreutils}/bin/printf '%s\n' ${lib.escapeShellArg (builtins.toJSON tokenSpecs)} \
        | ${pkgs.python3}/bin/python3 ${tokenMaterializer} -
    '';
}
