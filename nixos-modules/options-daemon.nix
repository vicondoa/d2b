{ lib, config, ... }:

let
  cfg = config.nixling;

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
      implementedDescription = "Daemon-only foundation shipped (broker socket-activation + bundle digest verify + canonical /run/nixling + nixlingd.service restartIfChanged=false)";
      validatedDescription = "Validated via tests/nixlingd-startup-smoke.sh on this host with evidence record";
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
      validatedDescription = "Validated via tests/daemon-autostart-smoke.sh + tests/vms-json-parity.sh + ownership-eval";
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
      implementedDescription = "first-run validation UX shipped (nixling host validate --apply + daemon auto-write on first op)";
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
  };

  readinessWaves = builtins.attrNames readinessWaveSpecs;
  every = predicate: values: lib.all predicate values;

  # Readiness-wave gate set retained from the historical default-flip
  # design: the subset of readiness waves that must ALL report
  # implemented + validated + evidence-file-present before the
  # daemon-only end state was considered fully attested. The set
  # intentionally excludes waves that legitimately shipped AFTER the
  # original flip (p5 first-run UX, p6 clean break, p7 release cut).
  # `nixling.daemonExperimental.enable` is now an obsolete always-true
  # compat gate (default `true`); this set no longer flips its default.
  # The per-wave `validated` evidence still gates the readiness
  # assertions below and `nixling host validate`. Related deliverables
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
  options.nixling.daemonExperimental = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      defaultText = lib.literalExpression ''
        true
      '';
      description = ''
        Obsolete compatibility gate for the daemon-backed control plane.
        The daemon-only end state is always enabled; consumers should
        not set this option.
      '';
    };

    defaultFlipEvidenceDir = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/nixling/validated";
      example = "/var/lib/nixling/validated";
      description = ''
        Filesystem directory holding the per-wave validation evidence
        files (`<wave>.json`) consumed by the per-wave
        `validated = true` eval assertion and by `nixling host
        validate`. (Historically these files also gated the
        `nixling.daemonExperimental.enable` default-flip; that gate is
        retired now that `daemonExperimental.enable` is an obsolete
        always-true compat gate.) The default `/var/lib/nixling/validated`
        is the canonical operator-host location; the option is
        overridable mainly for regression tests (see
        `tests/daemon-default-compat-eval.sh`).
      '';
    };
  };

  # Default-switch readiness now tracks shipped-vs-validated state
  # separately. `implemented` flips when the code lands; the
  # operator may only set `validated = true` after recording host-local
  # evidence under /var/lib/nixling/validated/<wave>.json.
  options.nixling.defaultSwitchReadiness = lib.mapAttrs mkReadinessOption readinessWaveSpecs;

  # nixlingd autostart contract knobs.
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
            + "(GPU/audio sidecars spawn through the SpawnRunner "
            + "broker exec).";
        }
        {
          assertion =
            (!cfg.defaultSwitchReadiness.w5Fu.validated)
            || cfg.defaultSwitchReadiness.w4Fu.validated;
          message =
            "nixling.defaultSwitchReadiness.w5Fu.validated = true requires "
            + "nixling.defaultSwitchReadiness.w4Fu.validated = true "
            + "(validation depends on the SpawnRunner path "
            + "already being validated).";
        }
      ];
  };
}
