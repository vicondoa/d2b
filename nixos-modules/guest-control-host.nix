{ config, pkgs, lib, ... }:

let
  cfg = config.nixling;
  nl = import ./lib.nix { inherit lib; };
  enabledGuestControlVms =
    lib.filterAttrs (_: vm: vm.enable && vm.guest.control.enable) cfg.vms;
  usernamePattern = "^[a-z][a-z0-9_-]{0,31}$";
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
      !vm.enable
      || !vm.usbip.yubikey
      || vm.guest.control.enable;
    message = ''
      nixling.vms.${name}.usbip.yubikey requires
      nixling.vms.${name}.guest.control.enable because guest-side USBIP
      attach/detach is owned by guestd over the authenticated guest-control
      plane. There is no SSH fallback.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion =
      !vm.guest.exec.enable
      || vm.ssh.user != null;
    message = ''
      nixling.vms.${name}.guest.exec.enable is true, but no workload user is
      configured. Guest exec always runs the command as the VM's workload user
      (never root); set nixling.vms.${name}.ssh.user to the in-guest user exec
      should run as.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion =
      !vm.guest.exec.enable
      || vm.ssh.user == null
      || (usernameValid vm.ssh.user && vm.ssh.user != "root");
    message = ''
      nixling.vms.${name}.ssh.user (the guest exec workload user) must match
      ${usernamePattern} and must not be root. Guest exec never runs as root;
      users elevate with sudo inside the session.
    '';
  }) cfg.vms;

  system.activationScripts.nixlingGuestControlTokens =
    lib.stringAfter [ "users" ] ''
      ${pkgs.coreutils}/bin/printf '%s\n' ${lib.escapeShellArg (builtins.toJSON tokenSpecs)} \
        | ${pkgs.python3}/bin/python3 ${tokenMaterializer} -
    '';
}
