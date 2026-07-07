# nix-unit cases for the privileged broker service cgroup posture.
#
# The broker is socket-activated but owns privileged runner spawn and
# cgroup-delegation operations. Its unit must stay in d2b.slice,
# must be explicitly delegated, and must not use systemd service teardown
# as the runner lifecycle mechanism.
{ mkEval, lib, ... }:

let
  minimal = { ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    d2b.daemonExperimental.enable = true;
  };

  cfg = (mkEval [ minimal ]).config;
  svcCfg = cfg.systemd.services.d2b-priv-broker.serviceConfig or { };
  sliceCfg = cfg.systemd.slices.d2b.sliceConfig or { };
  tmpfiles = cfg.systemd.tmpfiles.rules;
in
{
  "broker-service-posture/service-slice" = {
    expr = svcCfg.Slice or "";
    expected = "d2b.slice";
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

  "broker-service-posture/no-global-manager-environment-mutation" = {
    expr = {
      execStartPreAvoidsSetEnvironment =
        !(lib.hasInfix "set-environment" (svcCfg.ExecStartPre or ""));
      execStartAvoidsSetEnvironment =
        !(lib.hasInfix "set-environment" (svcCfg.ExecStart or ""));
      usesUnitLocalEnvironmentFile = svcCfg.EnvironmentFile or "";
      environmentFileParentNotGroupWritable =
        builtins.elem "d /run/d2b/broker 0750 root d2bd -" tmpfiles;
    };
    expected = {
      execStartPreAvoidsSetEnvironment = true;
      execStartAvoidsSetEnvironment = true;
      usesUnitLocalEnvironmentFile = "/run/d2b/broker/priv-broker.env";
      environmentFileParentNotGroupWritable = true;
    };
  };
}
