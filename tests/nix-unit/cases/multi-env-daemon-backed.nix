# nix-unit cases for the multi-env daemon-backed example variant.
#
# The example-level flake checks are covered by the root `eval-multi-env`
# and `eval-multi-env-daemon` checks. The value/introspection assertions
# below are reconstructed against the ROOT flake's module set via `mkEval`.
#
# Reconstructed variants:
#   * demo   = examples/multi-env/configuration.nix
#   * daemon = + the example flake's `multi-env-daemon-experimental` overlay
#              (site.allowUnsafeEastWest + per-env mtu/mssClamp/east-west on
#              the work env), exactly as examples/multi-env/flake.nix layers
#              it.
#
# Covered (value/introspection):
#   * host.json env-level v0.4.0 network-knob propagation for the daemon
#     variant: site.allowUnsafeEastWest, work {mtu, mssClamp, lan
#     allow/effective east-west} and per-role bridgePortFlags, plus the
#     `personal` negative control (stays isolated, no east-west opt-in).
#   * vms.json (manifest) carries no `microvm@work-app` / `nixling@work-app.`
#     systemd-unit reference.
#   * processes.json node-level systemd `unit` fields.
#
# Spec corrections ("existing code is canon" — ADR 0015 daemon-only):
#   1. The bash gate (steps 5-6) asserted a supervisor split: the daemon
#      variant's `work-app` drops node `unit` fields while the
#      systemd-supervised `personal-app` and the legacy `demo` variant
#      KEEP `microvm@<vm>.service` unit references. In v1.1 daemon-only the
#      `nixling.vms.<vm>.supervisor` option is removed and every enabled VM
#      is daemon-supervised, so the framework emits NO per-VM systemd unit
#      for ANY node of ANY VM in EITHER variant (verified by probe: every
#      node's `unit` is null). The cases below assert the real invariant —
#      zero node-level unit fields across both variants — superseding the
#      obsolete supervisor-split expectation.
#   2. The bash gate (step 5) also asserted processes.json contains no
#      `microvm@work-app` substring. The current code uses `microvm@<vm>`
#      as the cloud-hypervisor runner's process-label argv token (not a
#      systemd unit), so that substring legitimately appears in argv. The
#      meaningful, still-true invariant is migrated at the manifest level
#      (vms.json carries no such reference), not over the raw processes
#      argv.
#
# multi-env is graphics-free, so the cases contribute on every system and
# the asserted values are platform-independent.
{ mkEval, lib, flakeRoot, ... }:

let
  configMod = import (flakeRoot + "/examples/multi-env/configuration.nix");

  # Mirrors examples/multi-env/flake.nix's `multi-env-daemon-experimental`
  # overlay.
  daemonExtra = { lib, ... }: {
    nixling.site.allowUnsafeEastWest = true;
    nixling.daemonExperimental.enable = true;
    nixling.envs.work.mtu = lib.mkForce 1400;
    nixling.envs.work.mssClamp = lib.mkForce true;
    nixling.envs.work.lan.allowEastWest = lib.mkForce true;
  };

  demoCfg = (mkEval [ configMod ]).config;
  daemonCfg = (mkEval [ configMod daemonExtra ]).config;

  hostJson = builtins.fromJSON daemonCfg.nixling._bundle.hostJson.jsonText;
  envOf = name: builtins.head (builtins.filter (e: e.env == name) hostJson.environments);
  work = envOf "work";
  personal = envOf "personal";
  # Project the four bridge-port flag bits for one role (drops the verbose
  # `rule` prose the bash gate did not assert on).
  flags = env: role:
    let f = builtins.head (builtins.filter (x: x.role == role) env.bridgePortFlags);
    in { inherit (f) isolated neighSuppress learning unicastFlood; };

  daemonProcs = daemonCfg.nixling._bundle.processesJson.data;
  demoProcs = demoCfg.nixling._bundle.processesJson.data;
  vmNodes = procs: vm:
    (builtins.head (builtins.filter (v: v.vm == vm) procs.vms)).nodes;
  unitCount = procs: vm:
    builtins.length (builtins.filter (n: (n.unit or null) != null) (vmNodes procs vm));

  manifestText = daemonCfg.nixling._manifestPkg.text;
in
{
  # ---- host.json env-level propagation (daemon variant) ----
  "multi-env-daemon/site-allow-unsafe-east-west" = {
    expr = hostJson.site.allowUnsafeEastWest;
    expected = true;
  };
  "multi-env-daemon/work-mtu" = {
    expr = work.mtu;
    expected = 1400;
  };
  "multi-env-daemon/work-mss-clamp" = {
    expr = work.mssClamp;
    expected = 1360;
  };
  "multi-env-daemon/work-lan-allow-east-west" = {
    expr = work.lan.allowEastWest;
    expected = true;
  };
  "multi-env-daemon/work-lan-effective-east-west" = {
    expr = work.lan.effectiveEastWest;
    expected = true;
  };
  "multi-env-daemon/work-workload-lan-flags" = {
    expr = flags work "workload-lan";
    expected = { isolated = false; neighSuppress = false; learning = true; unicastFlood = true; };
  };
  "multi-env-daemon/work-net-vm-lan-flags" = {
    expr = flags work "net-vm-lan";
    expected = { isolated = false; neighSuppress = false; learning = true; unicastFlood = true; };
  };
  "multi-env-daemon/work-uplink-flags" = {
    expr = flags work "uplink";
    expected = { isolated = true; neighSuppress = true; learning = false; unicastFlood = false; };
  };

  # ---- personal env negative control (no east-west opt-in) ----
  "multi-env-daemon/personal-lan-effective-east-west" = {
    expr = personal.lan.effectiveEastWest;
    expected = false;
  };
  "multi-env-daemon/personal-workload-lan-flags" = {
    expr = flags personal "workload-lan";
    expected = { isolated = true; neighSuppress = true; learning = true; unicastFlood = false; };
  };

  # ---- vms.json (manifest) carries no per-VM systemd unit reference ----
  "multi-env-daemon/manifest-no-microvm-work-app" = {
    expr = lib.hasInfix "microvm@work-app" manifestText;
    expected = false;
  };
  "multi-env-daemon/manifest-no-nixling-work-app" = {
    expr = lib.hasInfix "nixling@work-app." manifestText;
    expected = false;
  };

  # ---- processes.json node-level systemd unit fields (ADR 0015) ----
  # Spec correction #1: daemon-only emits no per-VM systemd unit for any
  # node of any VM in either variant.
  "multi-env-daemon/daemon-work-app-unit-count" = {
    expr = unitCount daemonProcs "work-app";
    expected = 0;
  };
  "multi-env-daemon/daemon-personal-app-unit-count" = {
    expr = unitCount daemonProcs "personal-app";
    expected = 0;
  };
  "multi-env-daemon/demo-work-app-unit-count" = {
    expr = unitCount demoProcs "work-app";
    expected = 0;
  };
  "multi-env-daemon/demo-personal-app-unit-count" = {
    expr = unitCount demoProcs "personal-app";
    expected = 0;
  };
}
