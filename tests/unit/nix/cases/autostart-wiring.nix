# nix-unit cases migrated from tests/autostart-wiring-eval.sh.
#
# Positive-eval regression for the daemon-driven autostart wiring in the
# daemon-only end-state. After the `nixling@<vm>.service` template was
# deleted, autostart is driven by the `nixlingd.service` daemon (it reads
# `nixling.vms.<name>.autostart` out of the bundle and brings VMs up via
# the broker `SpawnRunner{role: CloudHypervisor}` op). This gate is the
# inverse of its pre-cutover form:
#
#   - `systemd.services."nixling@"` (and per-instance attrs) MUST NOT exist
#     (template deleted);
#   - `systemd.targets.multi-user.wants` MUST NOT pull any
#     `nixling@*.service` (dangling wants would log a load failure at boot);
#   - `nixlingd.service` MUST be wired into `multi-user.target.wants` so it
#     comes up on boot and drives `autostart = true` VMs;
#   - `systemd.targets.microvms.wants` MUST still be `[]` — the upstream
#     `microvm@<vm>` autostart cascade stays suppressed even though the
#     host.nix `microvm.vms` translation is preserved.
#
# Synthesizes one autostart=true VM and one autostart=false VM with the
# daemon explicitly enabled, then introspects the real host config via
# `mkEval`.
{ mkEval, lib, ... }:

let
  base = { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    nixling.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    nixling.daemonExperimental.enable = true;
    nixling.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    nixling.vms.auto-vm = {
      enable = true;
      env = "work";
      index = 10;
      autostart = true;
      ssh.user = "alice";
      config = { lib, ... }: {
        networking.hostName = lib.mkDefault "auto-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
    nixling.vms.manual-vm = {
      enable = true;
      env = "work";
      index = 11;
      autostart = false;
      ssh.user = "alice";
      config = { lib, ... }: {
        networking.hostName = lib.mkDefault "manual-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
  };

  cfg = (mkEval [ base ]).config;
  svcs = cfg.systemd.services;
  mu = cfg.systemd.targets.multi-user.wants;
  mvms =
    if builtins.hasAttr "microvms" cfg.systemd.targets
    then cfg.systemd.targets.microvms.wants or [ ]
    else [ ];
  nlAttrs = lib.filter (n: lib.hasPrefix "nixling@" n && lib.hasSuffix ".service" n) mu;
  nixlingdWantedBy =
    if builtins.hasAttr "nixlingd" svcs then svcs.nixlingd.wantedBy or [ ] else [ ];
in
{
  "autostart-wiring/no-nixling-template" = {
    expr = builtins.hasAttr "nixling@" svcs;
    expected = false;
  };
  "autostart-wiring/no-nixling-auto-vm" = {
    expr = builtins.hasAttr "nixling@auto-vm" svcs;
    expected = false;
  };
  "autostart-wiring/no-nixling-manual-vm" = {
    expr = builtins.hasAttr "nixling@manual-vm" svcs;
    expected = false;
  };
  "autostart-wiring/no-dangling-nixling-wants" = {
    expr = nlAttrs;
    expected = [ ];
  };
  "autostart-wiring/nixlingd-present" = {
    expr = builtins.hasAttr "nixlingd" svcs;
    expected = true;
  };
  "autostart-wiring/nixlingd-wired-multi-user" = {
    expr = builtins.elem "multi-user.target" nixlingdWantedBy;
    expected = true;
  };
  "autostart-wiring/microvms-target-empty" = {
    expr = mvms;
    expected = [ ];
  };
}
