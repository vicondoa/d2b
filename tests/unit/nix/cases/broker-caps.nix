# nix-unit cases migrated from tests/broker-caps-eval.sh.
#
# Asserts that systemd.services.nixling-priv-broker.serviceConfig
# .CapabilityBoundingSet matches the canonical broker bounding set EXACTLY
# (no additions, no omissions) and that AmbientCapabilities carries the
# sentinel empty-string entry that drops all ambient caps.
#
# The canonical 16-cap set is the per-child-role union the broker needs so
# capset(2) succeeds in spawned runners (CH/swtpm/gpu still require the
# union even though ADR 0021's broker-pre-NS model shrinks virtiofsd's
# per-spawn requirements). Notable hard-FAIL absences the exact-set match
# guards against: CAP_SYS_PTRACE, CAP_NET_BIND_SERVICE, CAP_AUDIT_WRITE.
#
# The bash gate's order-independent set comparison migrates to a single
# `lib.sort builtins.lessThan` value case against the sorted canonical list;
# the ambient-sentinel check migrates verbatim (list-or-string aware).
#
# Reading serviceConfig.{CapabilityBoundingSet,AmbientCapabilities} is lazy
# and does NOT force serviceConfig.ExecStart (which would recurse on the
# broker derivation) — the same projection the bash gate relied on.
{ mkEval, lib, ... }:

let
  # Canonical broker CapabilityBoundingSet, sorted. Per host-broker.nix.
  canonicalSorted = [
    "CAP_CHOWN"
    "CAP_DAC_OVERRIDE"
    "CAP_DAC_READ_SEARCH"
    "CAP_FOWNER"
    "CAP_FSETID"
    "CAP_IPC_LOCK"
    "CAP_KILL"
    "CAP_LEASE"
    "CAP_MKNOD"
    "CAP_NET_ADMIN"
    "CAP_NET_RAW"
    "CAP_SETFCAP"
    "CAP_SETGID"
    "CAP_SETPCAP"
    "CAP_SETUID"
    "CAP_SYS_ADMIN"
    "CAP_SYS_RESOURCE"
  ];

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
    nixling.daemonExperimental.enable = true;
  };

  brokerService = (mkEval [ base ]).config.systemd.services.nixling-priv-broker;
  svc = brokerService.serviceConfig;
  env = brokerService.environment;
  cbs = svc.CapabilityBoundingSet or null;
  ac = svc.AmbientCapabilities or null;
in
{
  "broker-caps/cbs-not-null" = {
    expr = cbs != null;
    expected = true;
  };
  "broker-caps/cbs-exact-canonical-set" = {
    expr = lib.sort builtins.lessThan cbs;
    expected = canonicalSorted;
  };
  "broker-caps/ambient-not-null" = {
    expr = ac != null;
    expected = true;
  };
  "broker-caps/ambient-empty-sentinel" = {
    expr = if builtins.isList ac then builtins.elem "" ac else ac == "";
    expected = true;
  };
  "broker-caps/default-rust-log-info" = {
    expr = env.RUST_LOG or null;
    expected = "info";
  };
}
