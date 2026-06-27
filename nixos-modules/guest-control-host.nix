{ config, pkgs, lib, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  enabledGuestControlVms =
    lib.filterAttrs (_: vm: vm.enable && vm.guest.control.enable) cfg.vms;
  usernamePattern = "^[a-z][a-z0-9_-]{0,31}$";
  usernameValid = user: builtins.match usernamePattern user != null;
  tokenSpecs = lib.mapAttrsToList (name: vm: {
    inherit name;
    source = vm.guest.control.auth.tokenFile;
    target = "${cfg.site.stateDir}/guest-control-${name}/token";
    readerGid = d2bLib.stablePrincipalId "d2b-${name}-gctlfs";
  }) enabledGuestControlVms;
  tokenMaterializer = ./guest-control-token-materialize.py;
in
{
  assertions = lib.mapAttrsToList (name: vm: {
    assertion = vm.guest.control.enable || vm.guest.control.auth.tokenFile == null;
    message = ''
      d2b.vms.${name}.guest.control.auth.tokenFile is set, but
      d2b.vms.${name}.guest.control.enable is false. Enable guest-control
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
      d2b.vms.${name}.guest.control.auth.tokenFile must be an absolute
      runtime path outside /nix/store. Do not use Nix path literals or
      flake-relative secret files; use a runtime secret path such as
      /run/secrets/d2b/${name}/guest-control-token.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion =
      !vm.guest.exec.enable
      || vm.guest.control.enable;
    message = ''
      d2b.vms.${name}.guest.exec.enable requires
      d2b.vms.${name}.guest.control.enable because exec policy is enforced
      by the guest-control plane.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion =
      !vm.enable
      || !vm.usbip.yubikey
      || vm.guest.control.enable;
    message = ''
      d2b.vms.${name}.usbip.yubikey requires
      d2b.vms.${name}.guest.control.enable because guest-side USBIP
      attach/detach is owned by guestd over the authenticated guest-control
      plane. There is no SSH fallback.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion =
      !vm.guest.exec.enable
      || vm.ssh.user != null;
    message = ''
      d2b.vms.${name}.guest.exec.enable is true, but no workload user is
      configured. Guest exec always runs the command as the VM's workload user
      (never root); set d2b.vms.${name}.ssh.user to the in-guest user exec
      should run as.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion =
      !vm.guest.exec.enable
      || vm.ssh.user == null
      || (usernameValid vm.ssh.user && vm.ssh.user != "root");
    message = ''
      d2b.vms.${name}.ssh.user (the guest exec workload user) must match
      ${usernamePattern} and must not be root. Guest exec never runs as root;
      users elevate with sudo inside the session.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion =
      !vm.guest.shell.enable
      || vm.guest.control.enable;
    message = ''
      d2b.vms.${name}.guest.shell.enable requires
      d2b.vms.${name}.guest.control.enable because persistent shell policy
      is enforced by the authenticated guest-control plane.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion =
      !vm.guest.shell.enable
      || vm.guest.exec.enable;
    message = ''
      d2b.vms.${name}.guest.shell.enable requires
      d2b.vms.${name}.guest.exec.enable because persistent shells reuse the
      guest-control exec terminal substrate and workload-user policy.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion =
      !vm.guest.shell.enable
      || vm.ssh.user != null;
    message = ''
      d2b.vms.${name}.guest.shell.enable is true, but no workload user is
      configured. Persistent shells run as the VM's workload user (never root);
      set d2b.vms.${name}.ssh.user.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion =
      !vm.guest.shell.enable
      || vm.ssh.user == null
      || (usernameValid vm.ssh.user && vm.ssh.user != "root");
    message = ''
      d2b.vms.${name}.ssh.user (the persistent shell workload user) must
      match ${usernamePattern} and must not be root.
    '';
  }) cfg.vms
  ++ lib.mapAttrsToList (name: vm: {
    assertion = vm.guest.shell.maxAttached <= vm.guest.shell.maxSessions;
    message = ''
      d2b.vms.${name}.guest.shell.maxAttached must be less than or equal to
      guest.shell.maxSessions.
    '';
  }) cfg.vms;

  system.activationScripts.d2bGuestControlTokens =
    lib.stringAfter [ "users" ] ''
      ${pkgs.coreutils}/bin/printf '%s\n' ${lib.escapeShellArg (builtins.toJSON tokenSpecs)} \
        | ${pkgs.python3}/bin/python3 ${tokenMaterializer} -
    '';
}
