{ lib, config, ... }:

let
  cfg = config.d2b;

  # the default-switch criteria
  # (docs/explanation/default-switch-and-deprecation.md) now track two
  # bits per remaining rollout wave
  #   - `implemented` = code shipped in-tree
  #   - `validated`   = recorded host-local evidence that the wave was
  #                     exercised successfully in production-like smoke
  #                     on this operator host
  readinessWaveSpecs = {
    w4Fu = {
      implementedDescription = "Headless daemon + supervisor path shipped";
      validatedDescription = "Validated via Ubuntu Tier-1 smoke + audit log evidence";
      implementedDefault = true;
    };
    w5Fu = {
      implementedDescription = "Minijail profiles + GPU/audio/video argv generators shipped";
      validatedDescription = "Validated via hardware smoke + audit log evidence";
      implementedDefault = true;
    };
    w6Fu = {
      implementedDescription = "USBIP live executors + per-busid lock shipped";
      validatedDescription = "Validated via hardware smoke + USBIP audit evidence";
      implementedDefault = true;
    };
    w7Fu = {
      implementedDescription = "Store-lifecycle verbs + admin auth shipped";
      validatedDescription = "Validated via switch/boot/test/rollback/gc smoke + audit log evidence";
      implementedDefault = true;
    };
    w8Fu = {
      implementedDescription = "Keys/trust/rotate-known-host live wiring shipped";
      validatedDescription = "Validated via keys/trust smoke + audit log evidence";
      implementedDefault = true;
    };
    w9Fu = {
      implementedDescription = "Host install + migrate live broker ops shipped";
      validatedDescription = "Validated via host install/migrate smoke + audit log evidence";
      implementedDefault = true;
    };
    p0 = {
      implementedDescription = "Daemon-only foundation shipped (broker socket-activation + bundle digest verify + canonical /run/d2b + notify-ready d2bd.service)";
      validatedDescription = "Validated via tests/d2bd-startup-smoke.sh on this host with evidence record";
      implementedDefault = false;
    };
    p0Fu = {
      implementedDescription = "Cgroup delegation sequence + bundle-tampered envelope + per-artifact hash verification + ListenSequentialPacket socket fix shipped";
      validatedDescription = "Validated via tests/broker-cgroup-delegation-smoke.sh";
      implementedDefault = false;
    };
    p1 = {
      implementedDescription = "per-role minijail profiles + byte-parity argv generators shipped (CH, Virtiofsd, Swtpm, Gpu, Audio, Video, VsockRelay, Usbip, OtelHostBridge)";
      validatedDescription = "Validated via per-role tests/minijail-validator-<role>.sh + hardware-smoke on NVIDIA Quadro T1000 / virtio-snd / virtio-media";
      implementedDefault = false;
    };
    p2 = {
      implementedDescription = "daemon-side host-prep + ownership matrix + manifestVersion=4 + daemon autostart shipped";
      validatedDescription = "Validated via tests/daemon-autostart-smoke.sh + tests/unit/gates/vms-json-parity.sh + ownership-eval";
      implementedDefault = false;
    };
    p3 = {
      implementedDescription = "host singletons retired (net-route-preflight, audit-check, ch-exporter, otel-host-bridge, per-env usbipd) + daemon health endpoint";
      validatedDescription = "Validated via tests/observability-eval.sh + USBIP smoke + degraded-mode escape-hatch smoke";
      implementedDefault = false;
    };
    p4 = {
      implementedDescription = "VM start/stop/restart/list daemon-native end-to-end; .desktop wrapper updated";
      validatedDescription = "Validated via per-VM vm start smoke + Wayland desktop launcher smoke";
      implementedDefault = false;
    };
    p5 = {
      implementedDescription = "first-run validation UX shipped (d2b host validate --apply + daemon auto-write on first op)";
      validatedDescription = "Validated via fresh-host bootstrap smoke";
      implementedDefault = false;
    };
    p6 = {
      implementedDescription = "legacy systemd template emission + bash CLI removed (clean break; v1.0 retains the supervisor option for backward-compat with consumer flakes pinning pre-v1.0 manifests, see ADR 0015 § Decision)";
      validatedDescription = "Validated via tests/legacy-unit-denylist-eval.sh + tests/static.sh green";
      implementedDefault = false;
    };
    p7 = {
      implementedDescription = "docs blast-radius + v1.0 cut shipped";
      validatedDescription = "Validated via static.sh + per-example flake-check green";
      implementedDefault = false;
    };
    p0Cb = {
      implementedDescription = "d2b-clipd clipboard authority foundation + picker protocol IPC (ADR 0042)";
      validatedDescription = "Validated via tests/clipboard-picker-smoke.sh with real d2b-clip-picker binary";
      implementedDefault = false;
    };
  };

  readinessWaves = builtins.attrNames readinessWaveSpecs;
  every = predicate: values: lib.all predicate values;

  # Readiness-wave gate set retained from the historical default-flip
  # design: the subset of readiness waves that must ALL report
  # implemented + validated + evidence-file-present before the
  # daemon-only end state was considered fully attested. The set
  # intentionally excludes waves that legitimately shipped AFTER the
  # original flip (p5 first-run UX, p6 clean break, p7 release cut).
  # `d2b.daemonExperimental.enable` now defaults `true` and is no
  # longer evidence-auto-flipped by this set, but it still functionally
  # gates the daemon control plane (setting it `false` reverts the host
  # to the unsupported pre-daemon legacy state); this set no longer
  # flips its default.
  # The per-wave `validated` evidence still gates the readiness
  # assertions below and `d2b host validate`. Related deliverables
  # are modelled inside the existing w4Fu (headless daemon and
  # supervisor path), w8Fu, and w9Fu (host install + migrate) readiness
  # records; w7Fu carries the store-lifecycle slice that landed
  # alongside them.
  flipGateWaves = [
    "w4Fu"
    "w5Fu"
    "w6Fu"
    "w7Fu"
    "w8Fu"
    "w9Fu"
    "p0"
    "p0Fu"
    "p1"
    "p2"
    "p3"
    "p4"
  ];

  validationEvidenceDir = cfg.daemonExperimental.defaultFlipEvidenceDir;
  validationEvidenceFileText = wave: "${validationEvidenceDir}/${wave}.json";
  validationEvidenceFile = wave: /. + (validationEvidenceFileText wave);

  mkReadinessOption = _: spec: {
    implemented = (lib.mkEnableOption spec.implementedDescription) // {
      default = spec.implementedDefault or false;
      defaultText = lib.literalExpression (if spec.implementedDefault or false then "true" else "false");
    };
    validated = lib.mkEnableOption spec.validatedDescription;
  };

  waveReadiness = wave: cfg.defaultSwitchReadiness.${wave};
  waveReady = wave:
    let
      readiness = waveReadiness wave;
    in
    readiness.implemented && readiness.validated;

  waveFullyReady = wave: waveReady wave && validationEvidencePresent wave;

  flipGateReady = every waveFullyReady flipGateWaves;

  validationEvidencePayload = wave:
    let
      file = validationEvidenceFile wave;
    in
    if builtins.pathExists file then
      builtins.tryEval (builtins.fromJSON (builtins.readFile file))
    else
      {
        success = false;
        value = null;
      };

  validationEvidencePresent = wave:
    let
      payloadAttempt = validationEvidencePayload wave;
      payload = if payloadAttempt.success then payloadAttempt.value else null;
    in
    payloadAttempt.success
    && builtins.isAttrs payload
    && payload ? wave
    && builtins.isString payload.wave
    && payload.wave == wave
    && payload ? timestamp
    && builtins.isString payload.timestamp
    && payload.timestamp != ""
    && payload ? operatorSignature
    && builtins.isString payload.operatorSignature
    && payload.operatorSignature != "";
in
{
  options.d2b.daemonExperimental = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      defaultText = lib.literalExpression ''
        true
      '';
      description = ''
        Master switch for the daemon-only control plane. Defaults to
        `true` and still functionally gates the daemon: setting it
        `false` reverts the host to the unsupported pre-daemon legacy
        state, so consumers should leave it at its default. Retained
        for compatibility; it is no longer evidence-auto-flipped (the
        per-wave `defaultSwitchReadiness.<wave>.validated` evidence
        gates the readiness assertions and `d2b host validate`
        separately).
      '';
    };

    defaultFlipEvidenceDir = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/d2b/validated";
      example = "/var/lib/d2b/validated";
      description = ''
        Filesystem directory holding the per-wave validation evidence
        files (`<wave>.json`) consumed by the per-wave
        `validated = true` eval assertion and by `d2b host
        validate`. (Historically these files also gated the
        `d2b.daemonExperimental.enable` default-flip; that gate is
        retired — `daemonExperimental.enable` now simply defaults
        `true` and is no longer evidence-auto-flipped.) The default
        `/var/lib/d2b/validated`
        is the canonical operator-host location; the option is
        overridable mainly for regression tests (see
        `tests/daemon-default-compat-eval.sh`).
      '';
    };
  };

  # Default-switch readiness now tracks shipped-vs-validated state
  # separately. `implemented` flips when the code lands; the
  # operator may only set `validated = true` after recording host-local
  # evidence under /var/lib/d2b/validated/<wave>.json.
  options.d2b.defaultSwitchReadiness = lib.mapAttrs mkReadinessOption readinessWaveSpecs;

  # d2bd autostart contract knobs.
  # The Rust implementation lives in
  # packages/d2bd/src/autostart.rs; the contract is described in
  # docs/reference/daemon-autostart.md.
  options.d2b.daemon.autostart = {
    parallelism = lib.mkOption {
      type = lib.types.ints.positive;
      default = 3;
      example = 4;
      description = ''
        Concurrency cap N for the d2bd autostart pass that runs
        on daemon startup. At most N VMs are started in parallel
        within each phase (net VMs first, then workloads). The
        daemon clamps values < 1 to 1.

        Net VM failures do NOT abort the sequence — workloads in the
        same env are marked `degraded` and the daemon continues
        serving status/doctor/audit. See
        docs/reference/daemon-autostart.md for the full contract.
      '';
    };
  };

  options.d2b.daemon.lifecycle.gracefulShutdown = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Whether d2bd should ask supported local VM providers to shut
        the guest OS down before falling back to host-side VMM
        termination. Enabled by default for Cloud Hypervisor/NixOS VMs
        and qemu-media VMs; per-VM options can opt a VM out or back in.
      '';
    };

    timeoutSeconds = lib.mkOption {
      type = lib.types.int;
      default = 90;
      example = 120;
      description = ''
        Default bounded wait, in seconds, for provider-aware graceful guest
        shutdown before d2bd uses the standard SIGTERM/SIGKILL VMM
        cleanup path. Values must be between 1 and 600 seconds so a typo
        cannot stretch host shutdown or reboot for hours.
      '';
    };
  };

  options.d2b.daemon.lifecycle.liveActivation = {
    timeoutSeconds = lib.mkOption {
      type = lib.types.int;
      default = 600;
      example = 1800;
      description = ''
        Default bounded wait, in seconds, for an authenticated in-guest
        live activation (`d2b switch` / `test` / `rollback`) to
        finish before d2bd reports a typed activation timeout.
        Identity-bound guests may need a larger value when user-manager
        activation waits for an operator-mediated provider flow such as
        Entra/Himmelblau hello/PIN. Values must be between 1 and 3600
        seconds.
      '';
    };
  };

  config = {
    assertions =
      (map (
        wave:
        let
          readiness = waveReadiness wave;
        in
        {
          assertion = (!readiness.validated) || readiness.implemented;
          message =
            "d2b.defaultSwitchReadiness.${wave}.validated = true requires "
            + "d2b.defaultSwitchReadiness.${wave}.implemented = true.";
        }
      ) readinessWaves)
      ++ (map (
        wave:
        let
          readiness = waveReadiness wave;
        in
        {
          assertion = (!readiness.validated) || validationEvidencePresent wave;
          message =
            "d2b.defaultSwitchReadiness.${wave}.validated = true requires "
            + validationEvidenceFileText wave
            + " to exist and contain JSON fields \"wave\" = \"${wave}\", "
            + "\"timestamp\", and \"operatorSignature\".";
        }
      ) readinessWaves)
      ++ [
        {
          assertion =
            cfg.daemon.lifecycle.gracefulShutdown.timeoutSeconds >= 1
            && cfg.daemon.lifecycle.gracefulShutdown.timeoutSeconds <= 600;
          message = ''
            d2b.daemon.lifecycle.gracefulShutdown.timeoutSeconds must be
            between 1 and 600 seconds. The upper bound keeps host shutdown
            and reboot bounded; use per-VM timeout overrides only within the
            same range.
          '';
        }
        {
          assertion =
            cfg.daemon.lifecycle.liveActivation.timeoutSeconds >= 1
            && cfg.daemon.lifecycle.liveActivation.timeoutSeconds <= 3600;
          message = ''
            d2b.daemon.lifecycle.liveActivation.timeoutSeconds must be
            between 1 and 3600 seconds. Use per-VM overrides for guests whose
            user-manager activation may legitimately wait on an operator-mediated
            identity flow.
          '';
        }
        {
          assertion =
            (!cfg.defaultSwitchReadiness.w5Fu.implemented)
            || cfg.defaultSwitchReadiness.w4Fu.implemented;
          message =
            "d2b.defaultSwitchReadiness.w5Fu.implemented = true requires "
            + "d2b.defaultSwitchReadiness.w4Fu.implemented = true "
            + "(GPU/audio sidecars spawn through the SpawnRunner "
            + "broker exec).";
        }
        {
          assertion =
            (!cfg.defaultSwitchReadiness.w5Fu.validated)
            || cfg.defaultSwitchReadiness.w4Fu.validated;
          message =
            "d2b.defaultSwitchReadiness.w5Fu.validated = true requires "
            + "d2b.defaultSwitchReadiness.w4Fu.validated = true "
            + "(validation depends on the SpawnRunner path "
            + "already being validated).";
        }
      ];
  };
}
