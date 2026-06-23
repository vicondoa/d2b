# nix-unit cases for the privileged broker service cgroup posture.
#
# The broker is socket-activated but owns privileged runner spawn and
# cgroup-delegation operations. Its unit must stay in nixling.slice,
# must be explicitly delegated, and must not use systemd service teardown
# as the runner lifecycle mechanism.
{ mkEval, ... }:

let
  minimal = { ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    nixling.daemonExperimental.enable = true;
  };

  cfg = (mkEval [ minimal ]).config;
  svcCfg = cfg.systemd.services.nixling-priv-broker.serviceConfig or { };
  sliceCfg = cfg.systemd.slices.nixling.sliceConfig or { };
in
{
  "broker-service-posture/service-slice" = {
    expr = svcCfg.Slice or "";
    expected = "nixling.slice";
  };

  "broker-service-posture/service-delegate" = {
    expr = svcCfg.Delegate or false;
    expected = true;
  };

  "broker-service-posture/service-kill-mode" = {
    expr = svcCfg.KillMode or "";
    expected = "process";
  };

  "broker-service-posture/slice-delegate-controllers" = {
    expr = sliceCfg.Delegate or "";
    expected = "cpu memory pids io cpuset";
  };
}
