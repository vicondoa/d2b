# nix-unit cases migrated from tests/autostart-wiring-eval.sh.
#
# Positive-eval regression for the daemon-driven autostart wiring in the
# daemon-only end-state. After the `d2b@<vm>.service` template was
# deleted, autostart is driven by the `d2bd.service` daemon (it reads
# `d2b.vms.<name>.autostart` out of the bundle and brings VMs up via
# the broker `SpawnRunner{role: CloudHypervisor}` op). This gate is the
# inverse of its pre-cutover form:
#
#   - `systemd.services."d2b@"` (and per-instance attrs) MUST NOT exist
#     (template deleted);
#   - `systemd.targets.multi-user.wants` MUST NOT pull any
#     `d2b@*.service` (dangling wants would log a load failure at boot);
#   - `d2bd.service` MUST be wired into `multi-user.target.wants` so it
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
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    d2b.daemonExperimental.enable = true;
    d2b.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    d2b.vms.auto-vm = {
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
    d2b.vms.manual-vm = {
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
  nlAttrs = lib.filter (n: lib.hasPrefix "d2b@" n && lib.hasSuffix ".service" n) mu;
  d2bdWantedBy =
    if builtins.hasAttr "d2bd" svcs then svcs.d2bd.wantedBy or [ ] else [ ];
in
{
  "autostart-wiring/no-d2b-template" = {
    expr = builtins.hasAttr "d2b@" svcs;
    expected = false;
  };
  "autostart-wiring/no-d2b-auto-vm" = {
    expr = builtins.hasAttr "d2b@auto-vm" svcs;
    expected = false;
  };
  "autostart-wiring/no-d2b-manual-vm" = {
    expr = builtins.hasAttr "d2b@manual-vm" svcs;
    expected = false;
  };
  "autostart-wiring/no-dangling-d2b-wants" = {
    expr = nlAttrs;
    expected = [ ];
  };
  "autostart-wiring/d2bd-present" = {
    expr = builtins.hasAttr "d2bd" svcs;
    expected = true;
  };
  "autostart-wiring/d2bd-wired-multi-user" = {
    expr = builtins.elem "multi-user.target" d2bdWantedBy;
    expected = true;
  };
  "autostart-wiring/microvms-target-empty" = {
    expr = mvms;
    expected = [ ];
  };
}
