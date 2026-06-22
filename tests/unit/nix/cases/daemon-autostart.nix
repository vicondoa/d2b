# nix-unit cases migrated from tests/daemon-autostart-eval.sh.
#
# Asserts the static surface of the nixlingd autostart contract:
#
#   1. The Rust autostart module exposes the documented public surface
#      (AutostartPlan / VmAutostartEntry / AutostartConfig /
#      AutostartReport / AutostartOutcome / Outcome / VmStarter /
#      build_autostart_plan / execute_autostart) and DEFAULT_PARALLELISM
#      agrees with the NixOS default (3).
#   2. The daemon's lib.rs publishes the module, invokes
#      `run_startup_autostart` on startup (so the contract isn't dead
#      code), ships the production `BrokerVmStarter`, and exposes the
#      `autostart_parallelism` config field.
#   3. The contract is documented in docs/reference/daemon-autostart.md
#      and cross-referenced from docs/reference/daemon-api.md.
#   4. The `nixling.daemon.autostart.parallelism` NixOS option defaults to
#      3 and honours an override.
#   5. The daemon lifecycle graceful-shutdown Nix options render into
#      daemon-config.json / vms.json and keep host-shutdown systemd timing
#      bounded.
#
# The bash gate's `grep -qF` source/doc checks migrate to pure
# `builtins.readFile` substring cases (no IFD, no cargo — the flake source
# is already in scope as `flakeRoot`). The matching is line-oriented to
# mirror `grep -F` exactly and to avoid `lib.hasInfix`'s whole-file
# `builtins.match ".*needle.*"` blowing the evaluator stack on large
# sources like the 500 KB `lib.rs`. The option default / override checks
# migrate to `mkEval` introspection.
{ mkEval, lib, flakeRoot, ... }:

let
  linesOf = rel: lib.splitString "\n" (builtins.readFile (flakeRoot + rel));
  autostartRs = linesOf "/packages/nixlingd/src/autostart.rs";
  libRs = linesOf "/packages/nixlingd/src/lib.rs";
  hostDaemonNix = linesOf "/nixos-modules/host-daemon.nix";
  optionsDaemonNix = linesOf "/nixos-modules/options-daemon.nix";
  autostartMd = linesOf "/docs/reference/daemon-autostart.md";
  apiMd = linesOf "/docs/reference/daemon-api.md";

  # Faithful `grep -F <needle>`: true iff some line contains the literal.
  has = lines: needle: {
    expr = lib.any (l: lib.hasInfix needle l) lines;
    expected = true;
  };

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

  parOf = overrides:
    (mkEval ([ base ] ++ overrides)).config.nixling.daemon.autostart.parallelism;
  cfgOf = overrides:
    (mkEval ([ base ] ++ overrides)).config;
  daemonJsonOf = overrides:
    builtins.fromJSON (cfgOf overrides).environment.etc."nixling/daemon-config.json".text;
  lifecycleHost = { lib, ... }: {
    nixling.vms.work-vm = {
      env = "work";
      index = 10;
      lifecycle.gracefulShutdown.timeoutSeconds = 45;
    };
    nixling.vms.media = {
      runtime.kind = "qemu-media";
      env = "work";
      index = 42;
      qemuMedia.source = {
        kind = "image-file";
        path = "/var/lib/nixling/images/installer.img";
        format = "raw";
      };
    };
  };
  lifecycleCfg = cfgOf [ lifecycleHost ];
  disabledLifecycleCfg = cfgOf [
    lifecycleHost
    ({ ... }: {
      nixling.daemon.lifecycle.gracefulShutdown.enable = false;
      nixling.vms.media.lifecycle.gracefulShutdown.enable = true;
    })
  ];
  failureMessages = overrides:
    let cfg = cfgOf overrides;
    in map (a: a.message) (lib.filter (a: !a.assertion) cfg.assertions);
  hasFailure = overrides: needle:
    lib.any (message: lib.hasInfix needle message) (failureMessages overrides);
in
{
  # (1) Rust public surface in autostart.rs.
  "daemon-autostart/rs-AutostartPlan" = has autostartRs "pub struct AutostartPlan";
  "daemon-autostart/rs-VmAutostartEntry" = has autostartRs "pub struct VmAutostartEntry";
  "daemon-autostart/rs-AutostartConfig" = has autostartRs "pub struct AutostartConfig";
  "daemon-autostart/rs-AutostartReport" = has autostartRs "pub struct AutostartReport";
  "daemon-autostart/rs-AutostartOutcome" = has autostartRs "pub struct AutostartOutcome";
  "daemon-autostart/rs-Outcome-enum" = has autostartRs "pub enum Outcome";
  "daemon-autostart/rs-VmStarter-trait" = has autostartRs "pub trait VmStarter";
  "daemon-autostart/rs-build-plan-fn" = has autostartRs "pub fn build_autostart_plan";
  "daemon-autostart/rs-execute-fn" = has autostartRs "pub async fn execute_autostart";
  "daemon-autostart/rs-default-parallelism-const" = has autostartRs "pub const DEFAULT_PARALLELISM";
  "daemon-autostart/rs-default-parallelism-eq-3" = has autostartRs "pub const DEFAULT_PARALLELISM: usize = 3;";

  # (2) Daemon wiring in lib.rs.
  "daemon-autostart/librs-pub-mod-autostart" = has libRs "pub mod autostart;";
  "daemon-autostart/librs-run-startup-autostart" = has libRs "run_startup_autostart";
  "daemon-autostart/librs-broker-vm-starter" = has libRs "struct BrokerVmStarter";
  "daemon-autostart/librs-config-parallelism-field" = has libRs "autostart_parallelism";
  "daemon-autostart/host-daemon-renders-parallelism" = has hostDaemonNix "autostartParallelism = cfg.daemon.autostart.parallelism;";

  # (3) Documentation surface.
  "daemon-autostart/doc-net-vms-first" = has autostartMd "Net VMs first";
  "daemon-autostart/doc-concurrency-cap" = has autostartMd "Concurrency cap";
  "daemon-autostart/doc-degraded" = has autostartMd "Degraded";
  "daemon-autostart/doc-idempotent" = has autostartMd "Idempotent";
  "daemon-autostart/doc-parallelism" = has autostartMd "parallelism";
  "daemon-autostart/doc-option-name" = has autostartMd "nixling.daemon.autostart";
  "daemon-autostart/api-cross-ref" = has apiMd "daemon-autostart";

  # (4) NixOS option default + override.
  "daemon-autostart/option-default-3" = {
    expr = parOf [ ({ ... }: { }) ];
    expected = 3;
  };
  "daemon-autostart/option-override-7" = {
    expr = parOf [ ({ ... }: { nixling.daemon.autostart.parallelism = 7; }) ];
    expected = 7;
  };

  # (5) Graceful shutdown lifecycle Nix surface.
  "daemon-lifecycle/graceful-option-default-enabled" = {
    expr = lifecycleCfg.nixling.daemon.lifecycle.gracefulShutdown.enable;
    expected = true;
  };
  "daemon-lifecycle/graceful-option-default-timeout" = {
    expr = lifecycleCfg.nixling.daemon.lifecycle.gracefulShutdown.timeoutSeconds;
    expected = 90;
  };
  "daemon-lifecycle/graceful-option-docs-upper-bound" =
    has optionsDaemonNix "between 1 and 600 seconds";
  "daemon-lifecycle/daemon-config-renders-autostart" = {
    expr = (daemonJsonOf [ ({ ... }: { nixling.daemon.autostart.parallelism = 5; }) ]).autostartParallelism;
    expected = 5;
  };
  "daemon-lifecycle/daemon-config-renders-timeout" = {
    expr = (daemonJsonOf [ ({ ... }: { nixling.daemon.lifecycle.gracefulShutdown.timeoutSeconds = 120; }) ]).gracefulShutdownTimeoutSeconds;
    expected = 120;
  };
  "daemon-lifecycle/manifest-workload-graceful-enabled" = {
    expr = lifecycleCfg.nixling.manifest."work-vm".lifecycle.gracefulShutdown.enable;
    expected = true;
  };
  "daemon-lifecycle/manifest-workload-timeout-override" = {
    expr = lifecycleCfg.nixling.manifest."work-vm".lifecycle.gracefulShutdown.timeoutSeconds;
    expected = 45;
  };
  "daemon-lifecycle/manifest-qemu-graceful-enabled" = {
    expr = lifecycleCfg.nixling.manifest.media.lifecycle.gracefulShutdown.enable;
    expected = true;
  };
  "daemon-lifecycle/global-disable-per-vm-opt-in" = {
    expr = {
      work = disabledLifecycleCfg.nixling.manifest."work-vm".lifecycle.gracefulShutdown.enable;
      media = disabledLifecycleCfg.nixling.manifest.media.lifecycle.gracefulShutdown.enable;
    };
    expected = { work = false; media = true; };
  };
  "daemon-lifecycle/timeout-stop-sec-uses-phase-maxima" = {
    expr = lifecycleCfg.systemd.services.nixlingd.serviceConfig.TimeoutStopSec;
    expected = "360s";
  };
  "daemon-lifecycle/execstop-uses-host-shutdown-hook" = {
    expr = lib.hasPrefix "+" lifecycleCfg.systemd.services.nixlingd.serviceConfig.ExecStop
      && lib.hasInfix "nixling-host-shutdown-hook" lifecycleCfg.systemd.services.nixlingd.serviceConfig.ExecStop;
    expected = true;
  };
  "daemon-lifecycle/broker-shutdown-ordering" = {
    expr =
      builtins.elem "nixling-priv-broker.service" lifecycleCfg.systemd.services.nixlingd.after
      && builtins.elem "nixling-priv-broker.socket" lifecycleCfg.systemd.services.nixlingd.after
      && builtins.elem "dbus.service" lifecycleCfg.systemd.services.nixlingd.after;
    expected = true;
  };
  "daemon-lifecycle/global-timeout-upper-bound-assertion" = {
    expr = hasFailure
      [ ({ ... }: { nixling.daemon.lifecycle.gracefulShutdown.timeoutSeconds = 601; }) ]
      "nixling.daemon.lifecycle.gracefulShutdown.timeoutSeconds must be";
    expected = true;
  };
  "daemon-lifecycle/per-vm-timeout-lower-bound-assertion" = {
    expr = hasFailure
      [
        ({ ... }: {
          nixling.vms.work-vm = {
            env = "work";
            index = 10;
            lifecycle.gracefulShutdown.timeoutSeconds = 0;
          };
        })
      ]
      "nixling.vms.work-vm.lifecycle.gracefulShutdown.timeoutSeconds must be";
    expected = true;
  };
}
