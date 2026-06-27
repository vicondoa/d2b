# nix-unit cases migrated from tests/guest-config-containment-eval.sh.
#
# Eval-time containment gate for the per-VM guest-editable
# `d2b.vms.<vm>.guestConfigFile`. That file is the guest-editable OS
# layer (the surface the in-VM config-sync workflow edits) and MUST be
# CONTAINED: it may set only guest OS options, never host-owned
# `microvm.*` / `d2b.*` options. `assertions.nix` enforces this with a
# hard assertion driven by `lib.nix`'s `guestConfigForbiddenNamespaces`
# check, which evaluates the guest file over the real NixOS module set
# (with `microvm` / `d2b` redeclared as detector options) and reports
# any forbidden namespace the guest defines — by DEFINITION-EXISTENCE, so
# imports / `builtins.toFile`-generated modules / `_file` spoofing are all
# caught.
#
# Each fixture under tests/unit/nix/eval-cases/guest-fixtures/ is wired into a
# minimal consumer-style nixosSystem's corp-vm via `mkEval`, then the
# failing-assertion messages are introspected directly. Because the
# containment violation surfaces as a FAILING ASSERTION (a value), not an
# eval throw, the substring naming the offending options is preserved
# (faithful to the bash gate's `grep` checks) using `lib.hasInfix` over the
# joined messages — stronger than the harness's throw-only `expectedError`
# bucket would allow. The two contained fixtures assert the full failing
# list is empty, exactly as the bash gate's `[ "$out" = "[]" ]` check did.
#
# Graphics-free (corp-vm is headless), so no aarch64 platform guard is
# required.
{ mkEval, lib, flakeRoot, ... }:

let
  mkHost = fixture: { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    d2b.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    d2b.vms.corp-vm = {
      enable = true;
      env = "work";
      index = 10;
      ssh.user = "alice";
      config = {
        networking.hostName = lib.mkDefault "corp-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
      guestConfigFile = fixture;
    };
  };

  fixturePath = name: flakeRoot + "/tests/unit/nix/eval-cases/guest-fixtures/${name}";

  failingMessages = name:
    let cfg = (mkEval [ (mkHost (fixturePath name)) ]).config;
    in map (a: a.message) (builtins.filter (a: !a.assertion) cfg.assertions);

  joined = name: lib.concatStringsSep "\n" (failingMessages name);
in
{
  # --- clean guest config: NO failing assertion ----------------------
  "guest-config-containment/clean-no-failure" = {
    expr = failingMessages "clean-guest.nix";
    expected = [ ];
  };

  # --- guest READS a standard option: must NOT false-positive/crash --
  "guest-config-containment/reads-standard-option-no-failure" = {
    expr = failingMessages "reads-standard-option.nix";
    expected = [ ];
  };

  # --- guest sets microvm.*: rejected, naming the options ------------
  "guest-config-containment/sets-microvm-fires" = {
    expr = lib.hasInfix "may only set" (joined "sets-microvm.nix");
    expected = true;
  };
  "guest-config-containment/sets-microvm-names-mem" = {
    expr = lib.hasInfix "microvm.mem" (joined "sets-microvm.nix");
    expected = true;
  };
  "guest-config-containment/sets-microvm-names-cloud-hypervisor" = {
    expr = lib.hasInfix "microvm.cloud-hypervisor" (joined "sets-microvm.nix");
    expected = true;
  };

  # --- guest sets d2b.*: rejected, naming the option ------------
  "guest-config-containment/sets-d2b-fires" = {
    expr = lib.hasInfix "may only set" (joined "sets-d2b.nix");
    expected = true;
  };
  "guest-config-containment/sets-d2b-names-ssh-user" = {
    expr = lib.hasInfix "d2b.sshUser" (joined "sets-d2b.nix");
    expected = true;
  };

  # --- BYPASS #1: forbidden option via an imported module -----------
  "guest-config-containment/imports-microvm-fires" = {
    expr = lib.hasInfix "may only set" (joined "imports-microvm.nix");
    expected = true;
  };
  "guest-config-containment/imports-microvm-names-mem" = {
    expr = lib.hasInfix "microvm.mem" (joined "imports-microvm.nix");
    expected = true;
  };

  # --- BYPASS #2: forbidden option via a builtins.toFile module ------
  "guest-config-containment/tofile-microvm-fires" = {
    expr = lib.hasInfix "may only set" (joined "tofile-microvm.nix");
    expected = true;
  };
  "guest-config-containment/tofile-microvm-names-mem" = {
    expr = lib.hasInfix "microvm.mem" (joined "tofile-microvm.nix");
    expected = true;
  };

  # --- BYPASS #3: forbidden option with a spoofed module `_file` -----
  "guest-config-containment/spoof-file-fires" = {
    expr = lib.hasInfix "may only set" (joined "spoof-file.nix");
    expected = true;
  };
  "guest-config-containment/spoof-file-names-mem" = {
    expr = lib.hasInfix "microvm.mem" (joined "spoof-file.nix");
    expected = true;
  };
}
