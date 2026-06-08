{ lib, config, ... }:

let
  cfg = config.nixling;

  # W18 (W10-fu): the default-switch criteria
  # (docs/explanation/default-switch-and-deprecation.md) now track two
  # bits per remaining rollout wave:
  #   - `implemented` = code shipped in-tree
  #   - `validated`   = recorded host-local evidence that the wave was
  #                     exercised successfully in production-like smoke
  #                     on this operator host
  readinessWaveSpecs = {
    w4Fu = {
      implementedDescription = "W4-fu code shipped (W12/W14 headless daemon + supervisor path)";
      validatedDescription = "W4-fu validated via Ubuntu Tier-1 smoke + audit log evidence";
      implementedDefault = true;
    };
    w5Fu = {
      implementedDescription = "W5-fu code shipped (W17 minijail profiles + GPU/audio/video argv generators)";
      validatedDescription = "W5-fu validated via W20 hardware smoke + audit log evidence";
      implementedDefault = true;
    };
    w6Fu = {
      implementedDescription = "W6-fu code shipped (W13 USBIP live executors + per-busid lock)";
      validatedDescription = "W6-fu validated via W20 hardware smoke + USBIP audit evidence";
      implementedDefault = true;
    };
    w7Fu = {
      implementedDescription = "W7-fu code shipped (W7b store-lifecycle verbs + admin auth)";
      validatedDescription = "W7-fu validated via switch/boot/test/rollback/gc smoke + audit log evidence";
      implementedDefault = true;
    };
    w8Fu = {
      implementedDescription = "W8-fu code shipped (W14 keys/trust/rotate-known-host live wiring)";
      validatedDescription = "W8-fu validated via keys/trust smoke + audit log evidence";
      implementedDefault = true;
    };
    w9Fu = {
      implementedDescription = "W9-fu code shipped (W15 host install + migrate live broker ops)";
      validatedDescription = "W9-fu validated via host install/migrate smoke + audit log evidence";
      implementedDefault = true;
    };
    p0 = {
      implementedDescription = "P0 daemon-only foundation shipped (broker socket-activation + bundle digest verify + canonical /run/nixling + nixlingd.service restartIfChanged=false)";
      validatedDescription = "P0 validated via tests/nixlingd-startup-smoke.sh on this host with evidence record";
      implementedDefault = false;
    };
    p0Fu = {
      implementedDescription = "P0fu: cgroup delegation sequence + bundle-tampered envelope + per-artifact hash verification + ListenSequentialPacket socket fix";
      validatedDescription = "P0fu validated via tests/broker-cgroup-delegation-smoke.sh";
      implementedDefault = false;
    };
    p1 = {
      implementedDescription = "P1 per-role minijail profiles + byte-parity argv generators shipped (CH, Virtiofsd, Swtpm, Gpu, Audio, Video, VsockRelay, Usbip, OtelHostBridge)";
      validatedDescription = "P1 validated via per-role tests/minijail-validator-<role>.sh + hardware-smoke on NVIDIA Quadro T1000 / virtio-snd / virtio-media";
      implementedDefault = false;
    };
    p2 = {
      implementedDescription = "P2 daemon-side host-prep + ownership matrix + manifestVersion=3 + daemon autostart shipped";
      validatedDescription = "P2 validated via tests/daemon-autostart-smoke.sh + tests/vms-json-parity.sh + ownership-eval";
      implementedDefault = false;
    };
    p3 = {
      implementedDescription = "P3 host singletons retired (net-route-preflight, audit-check, ch-exporter, otel-host-bridge, per-env usbipd) + daemon health endpoint";
      validatedDescription = "P3 validated via tests/observability-eval.sh + USBIP smoke + degraded-mode escape-hatch smoke";
      implementedDefault = false;
    };
    p4 = {
      implementedDescription = "P4 vm start/stop/restart/list daemon-native end-to-end; .desktop wrapper updated";
      validatedDescription = "P4 validated via per-VM vm start smoke + Wayland desktop launcher smoke";
      implementedDefault = false;
    };
    p5 = {
      implementedDescription = "P5 first-run validation UX shipped (nixling host validate --apply + daemon auto-write on first op)";
      validatedDescription = "P5 validated via fresh-host bootstrap smoke";
      implementedDefault = false;
    };
    p6 = {
      implementedDescription = "P6 legacy systemd template emission + bash CLI removed (clean break; the planned `ph6-p6-supervisor-removed-assertion` is deferred to v1.1 backlog — v1.0 retains the supervisor option for backward-compat with consumer flakes pinning pre-v1.0 manifests, see ADR 0015 § Decision)";
      validatedDescription = "P6 validated via tests/legacy-unit-denylist-eval.sh + tests/static.sh green";
      implementedDefault = false;
    };
    p7 = {
      implementedDescription = "P7 docs blast-radius + v1.0 cut shipped";
      validatedDescription = "P7 validated via static.sh + per-example flake-check green";
      implementedDefault = false;
    };
  };

  readinessWaves = builtins.attrNames readinessWaveSpecs;
  every = predicate: values: lib.all predicate values;

  # W18 default-flip gate: the subset of readiness waves that, when
  # ALL report implemented + validated + evidence-file-present, make
  # `nixling.daemonExperimental.enable` default to `true`. The set
  # intentionally excludes waves that legitimately ship AFTER the
  # flip itself (p5 first-run UX, p6 clean break, p7 release cut);
  # the flip must be able to happen at P5 without waiting on its own
  # successors. The W12 / W14 / W15 / W15-fu1 waves named in the
  # phase plan are modelled inside the existing w4Fu (W12/W14
  # supervisor path), w8Fu (W14 keys/trust), and w9Fu (W15 host
  # install + migrate) readiness records; w7Fu carries the W7b
  # store-lifecycle slice that landed alongside them.
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
  options.nixling.daemonExperimental = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = flipGateReady;
      defaultText = lib.literalExpression ''
        true when every readiness wave in the W18 flip gate set
        (w4Fu, w5Fu, w6Fu, w7Fu, w8Fu, w9Fu, p0, p0Fu, p1, p2, p3, p4)
        has both .implemented and .validated set to true AND a matching
        evidence file at <defaultFlipEvidenceDir>/<wave>.json exists;
        otherwise false. Operator overrides (mkDefault/mkForce) win
        either way.
      '';
      description = ''
        Enable the daemon-backed control plane (nixlingd + per-VM
        supervisor + broker live wiring). W18 (P5 w18-flip): defaults
        to true on hosts where every wave in the W18 flip gate set
        (w4Fu..w9Fu plus the daemon-only rollout waves p0..p4)
        reports `implemented = true`, `validated = true`, AND has a
        matching `<defaultFlipEvidenceDir>/<wave>.json` evidence
        record. Otherwise defaults to false (legacy systemd-owned
        per-VM units).

        The future waves p5/p6/p7 are intentionally NOT part of the
        flip gate: the W18 flip happens at P5, before those waves
        ship, so requiring them would deadlock the auto-flip.

        Operators can override either way explicitly — `mkDefault`
        and `mkForce` semantics are preserved, so an explicit `= true`
        or `= false` pins the value regardless of the readiness +
        evidence computation.
      '';
    };

    defaultFlipEvidenceDir = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/nixling/validated";
      example = "/var/lib/nixling/validated";
      description = ''
        Filesystem directory holding the per-wave validation evidence
        files (`<wave>.json`) consumed by the W18 default-flip gate
        and by the per-wave `validated = true` eval assertion. The
        default `/var/lib/nixling/validated` is the canonical
        operator-host location; the option is overridable mainly for
        regression tests (see `tests/w18-default-flip-eval.sh`).
      '';
    };
  };

  # W10-fu default-switch readiness now tracks shipped-vs-validated
  # state separately. `implemented` flips when the code lands; the
  # operator may only set `validated = true` after recording host-local
  # evidence under /var/lib/nixling/validated/<wave>.json.
  options.nixling.defaultSwitchReadiness = lib.mapAttrs mkReadinessOption readinessWaveSpecs;

  # P2 ph2-p2-daemon-autostart: nixlingd autostart contract knobs.
  # The Rust implementation lives in
  # packages/nixlingd/src/autostart.rs; the contract is described in
  # docs/reference/daemon-autostart.md.
  options.nixling.daemon.autostart = {
    parallelism = lib.mkOption {
      type = lib.types.ints.positive;
      default = 3;
      example = 4;
      description = ''
        Concurrency cap N for the nixlingd autostart pass that runs
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
            "nixling.defaultSwitchReadiness.${wave}.validated = true requires "
            + "nixling.defaultSwitchReadiness.${wave}.implemented = true.";
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
            "nixling.defaultSwitchReadiness.${wave}.validated = true requires "
            + validationEvidenceFileText wave
            + " to exist and contain JSON fields \"wave\" = \"${wave}\", "
            + "\"timestamp\", and \"operatorSignature\".";
        }
      ) readinessWaves)
      ++ [
        {
          assertion =
            (!cfg.defaultSwitchReadiness.w5Fu.implemented)
            || cfg.defaultSwitchReadiness.w4Fu.implemented;
          message =
            "nixling.defaultSwitchReadiness.w5Fu.implemented = true requires "
            + "nixling.defaultSwitchReadiness.w4Fu.implemented = true "
            + "(GPU/audio sidecars spawn through the W4-fu SpawnRunner "
            + "broker exec).";
        }
        {
          assertion =
            (!cfg.defaultSwitchReadiness.w5Fu.validated)
            || cfg.defaultSwitchReadiness.w4Fu.validated;
          message =
            "nixling.defaultSwitchReadiness.w5Fu.validated = true requires "
            + "nixling.defaultSwitchReadiness.w4Fu.validated = true "
            + "(W5-fu validation depends on the W4-fu SpawnRunner path "
            + "already being validated).";
        }
      ];
  };
}
