# nix-unit cases migrated from tests/readiness-waves-eval.sh.
#
# Eval-time gate that the p0..p7 daemon-only rollout waves are present in
# the `nixling.defaultSwitchReadiness` option schema, that p0 defaults to
# implemented=false / validated=false (so the daemon doesn't auto-attest a
# wave that hasn't shipped + been validated), and that the obsolete
# `nixling.daemonExperimental.enable` compatibility option now defaults
# `true` (the daemon-only control plane is always enabled; see ADR 0015).
#
# Uses `mkEval` (== nixosSystem with the nixling module set) over a minimal
# host config — no VMs required — and reads the rendered
# `config.nixling.defaultSwitchReadiness` attrset directly. The bash gate
# hardcoded system = "x86_64-linux"; these cases are schema/value
# assertions whose results are platform-independent, so they contribute to
# the nix-unit check on every system unguarded.
{ mkEval, ... }:

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
    nixling.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
  };

  cfg = (mkEval [ base ]).config;
  rw = cfg.nixling.defaultSwitchReadiness;
  waveNames = builtins.attrNames rw;
  hasWave = w: builtins.elem w waveNames;
in
{
  "readiness-waves/has-p0" = { expr = hasWave "p0"; expected = true; };
  "readiness-waves/has-p1" = { expr = hasWave "p1"; expected = true; };
  "readiness-waves/has-p2" = { expr = hasWave "p2"; expected = true; };
  "readiness-waves/has-p3" = { expr = hasWave "p3"; expected = true; };
  "readiness-waves/has-p4" = { expr = hasWave "p4"; expected = true; };
  "readiness-waves/has-p5" = { expr = hasWave "p5"; expected = true; };
  "readiness-waves/has-p6" = { expr = hasWave "p6"; expected = true; };
  "readiness-waves/has-p7" = { expr = hasWave "p7"; expected = true; };

  # p0 defaults to implemented=false so the daemon doesn't auto-attest a
  # phase until it is explicitly shipped and validated.
  "readiness-waves/p0-implemented-default" = {
    expr = rw.p0.implemented;
    expected = false;
  };
  "readiness-waves/p0-validated-default" = {
    expr = rw.p0.validated;
    expected = false;
  };

  # daemonExperimental.enable is now an obsolete compatibility option whose
  # default is true; the daemon-only control plane is always enabled.
  "readiness-waves/daemon-auto-default" = {
    expr = cfg.nixling.daemonExperimental.enable;
    expected = true;
  };
}
