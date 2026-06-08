# nixos-modules/manifest.nix — typed JSON manifest contract.
#
# Builds the per-VM JSON manifest that the (current bash, future Rust)
# nixling CLI consumes at runtime. The manifest is the stable contract
# between the Nix-evaluated framework state and the imperative CLI; it
# carries every piece of per-VM metadata the CLI needs at command
# dispatch time (socket paths, IPs, env membership, capability flags,
# SSH credentials, …).
#
# Why an externally-typed module instead of an ad-hoc let-binding in
# cli.nix:
#
#   1. The JSON file at `/run/current-system/sw/share/nixling/vms.json`
#      is the integration surface for the Rust CLI port (Phase 8). It
#      must be documented and versioned. A typed `mkOption` gives us
#      a schema we can hand-walk into `docs/reference/manifest-schema.{md,json}`
#      and validate against in `tests/static.sh`.
#   2. The Nix module system catches schema regressions at eval time:
#      if a future refactor accidentally produces a field of the wrong
#      type, evaluation fails immediately rather than silently shipping
#      a broken JSON file.
#   3. The CLI can consume `config.nixling.manifest` directly from
#      sibling modules (e.g. `cli.nix`'s per-VM Konsole launcher) with
#      type-checked attribute access, no second `lib.mapAttrs` of the
#      same data.
#
# The JSON file's top-level layout is:
#
#   {
#     "_manifest": { "manifestVersion": <int> },
#     "<vmName>":  { name: ..., env: ..., apiSocket: ..., ... },
#     "<vmName>":  { ... },
#     ...
#   }
#
# `_manifest` is a reserved sentinel key — leading underscore disqualifies
# it as a VM name (the assertions.nix `vmNameOk` regex requires a
# leading lowercase letter), so the wrapper is non-colliding for any
# valid consumer config. All CLI jq filters use `--arg n` + `.[$n]`
# lookups which already skip non-matching keys, so reserved-key
# additions are forward-compatible.
#
# Schema-version contract: the integer `manifestVersion` is bumped on
# every breaking schema change (field removed, field renamed, field
# type narrowed, semantics changed). Additive changes (new optional
# fields) do not require a bump. See docs/reference/manifest-schema.md for the
# compatibility policy.
{ lib, pkgs, config, ... }:

let
  nl = import ./lib.nix { inherit lib pkgs; };
  inherit (nl) subnetIp;

  envMeta = config.nixling._envMeta;
  enabledVms = lib.filterAttrs (_: vm: vm.enable) config.nixling.vms;

  netVmOfEnv = envName:
    let n = config.nixling.envs.${envName}.netName or "sys-${envName}-net";
    in n;

  envOfNetVm = name:
    lib.findFirst
      (e: netVmOfEnv e == name)
      null
      (lib.attrNames config.nixling.envs);

  vmMeta = name: vm:
    let
      env = vm.env;
      m = if env != null && envMeta ? ${env} then envMeta.${env} else null;
      asNetVmForEnv = envOfNetVm name;
      derivedIp =
        if m != null then subnetIp m.lanSubnet vm.index
        else if asNetVmForEnv != null && envMeta ? ${asNetVmForEnv}
        then envMeta.${asNetVmForEnv}.netUplinkIp
        else null;
      derivedTap =
        if m != null then "${env}-l${toString vm.index}"
        else if asNetVmForEnv != null
        then "${asNetVmForEnv}-u2"
        else "vm-${name}";
      derivedBridge =
        if m != null then m.lanBridge
        else if asNetVmForEnv != null
        then envMeta.${asNetVmForEnv}.uplinkBridge
        else null;
      usbipdHostIp =
        if m != null then m.hostUplinkIp
        else null;
    in
    {
      inherit name;
      graphics = vm.graphics.enable;
      tpm = vm.tpm.enable;
      usbipYubikey = vm.usbip.yubikey;
      audio = vm.audio.enable;
      tap = derivedTap;
      bridge = derivedBridge;
      env = if env != null then env else asNetVmForEnv;
      isNetVm = asNetVmForEnv != null;
      netVm = if env != null then netVmOfEnv env else null;
      usbipdHostIp = usbipdHostIp;
      stateDir = "/var/lib/nixling/vms/${name}";
      apiSocket = "/var/lib/nixling/vms/${name}/${name}.sock";
      gpuSocket = "/var/lib/nixling/vms/${name}/${name}-gpu.sock";
      tpmSocket = "/run/swtpm/${name}/sock";
      # security-2: state file under root-owned non-group-writable subdir.
      audioStateFile = "/var/lib/nixling/vms/${name}/state/audio-state.json";
      audioService = "nixling-${name}-snd.service";
      staticIp =
        if derivedIp != null then derivedIp
        else vm.staticIp;
      sshUser = vm.ssh.user;
      # W4-followup H2 (security): `sshKeyPath` is intentionally NOT
      # part of the public manifest. The manifest ships to
      # `/run/current-system/sw/share/nixling/vms.json` which is
      # world-readable; exposing a per-VM private-key path there
      # leaks the location of secret material to every local user.
      # The CLI resolves the private-key path locally from
      # `config.nixling.site.keysDir` (or `vm.ssh.keyPath` when the
      # consumer overrides it) at Nix-eval time and bakes the
      # per-VM mapping into the shell wrapper. Consumers
      # reimplementing the CLI should mirror that — read
      # `nixling.site.keysDir` from their own privileged config
      # access, not from this world-readable file. The PUBLIC key
      # is fine to expose; if a future use case warrants it, add
      # `sshPubKeyPath` here.
    };

  computedManifest = lib.mapAttrs vmMeta enabledVms;

  # Top-level JSON shape: per-VM entries side-by-side with the
  # reserved `_manifest` schema-version sentinel. The CLI's jq
  # filters all use `--arg n` + `.[$n]` lookups, so adding the
  # sentinel does not affect per-VM iteration patterns.
  manifestJson = computedManifest // {
    _manifest = {
      manifestVersion = config.nixling._manifestVersion;
    };
  };

  manifestPkg = pkgs.writeTextFile {
    name = "nixling-vms-manifest";
    text = builtins.toJSON manifestJson;
    destination = "/share/nixling/vms.json";
  };

  # Per-VM submodule type, matching docs/reference/manifest-schema.json. Every
  # field is declared with a concrete type; the module system fails the
  # eval if the computed assignment from `vmMeta` ever drifts (e.g. a
  # refactor returns `null` for a field declared `str`).
  manifestEntryType = lib.types.submodule ({ name, ... }: {
    options = {
      name = lib.mkOption {
        type = lib.types.str;
        description = "VM name (attribute key in nixling.vms.<name>).";
      };

      graphics = lib.mkOption {
        type = lib.types.bool;
        description = ''
          True iff `nixling.vms.<name>.graphics.enable` is set. The CLI
          uses this to pick the launch path (interactive Konsole wrapper
          vs. `systemctl start microvm@<name>`).
        '';
      };

      tpm = lib.mkOption {
        type = lib.types.bool;
        description = "True iff `nixling.vms.<name>.tpm.enable` is set.";
      };

      usbipYubikey = lib.mkOption {
        type = lib.types.bool;
        description = ''
          True iff `nixling.vms.<name>.usbip.yubikey` is set. The CLI's
          `nixling usb` subcommand refuses to run for VMs where this is
          false.
        '';
      };

      audio = lib.mkOption {
        type = lib.types.bool;
        description = ''
          True iff `nixling.vms.<name>.audio.enable` is set. Live audio
          grant state lives in `audioStateFile`, not here — this flag
          only carries the capability bit.
        '';
      };

      tap = lib.mkOption {
        type = lib.types.str;
        description = ''
          Host-side tap-device name attached to the VM's net interface.
          Derived from `(env, index)` for env-attached workload VMs
          (`<env>-l<index>`), from the env's netVm role for net VMs
          (`<env>-u2`), or `vm-<name>` for legacy hand-rolled VMs.
        '';
      };

      bridge = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          Host-side Linux bridge the tap is attached to. Workload VMs
          use the env's `lanBridge` (`br-<env>-lan`); the net VM uses
          the env's `uplinkBridge` (`br-<env>-up`); legacy VMs have
          `bridge = null` (the consumer wires it themselves via
          `microvm.interfaces`).
        '';
      };

      env = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          Env this VM is in, or null for legacy hand-rolled VMs. For
          net VMs, this is the env they SERVE (not the env they're
          IN — net VMs are themselves the env's gateway).
        '';
      };

      isNetVm = lib.mkOption {
        type = lib.types.bool;
        description = ''
          True iff this VM is the auto-generated `sys-<env>-net` for
          some env. The CLI uses this to pick the bring-up order
          (net VMs first) and to skip net VMs in `nixling up <env>`
          batch operations the same way it skips `_manifest`.
        '';
      };

      netVm = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          For workload VMs: name of the net VM serving this VM's env.
          Null for net VMs themselves and for legacy hand-rolled VMs.
        '';
      };

      usbipdHostIp = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          For workload VMs: IP of the per-env usbipd proxy
          (`nixling-sys-<env>-usbipd-proxy.service`). The `nixling
          usb` subcommand passes this as `-r <ip>` to `usbip attach`.
          Null for net VMs and legacy VMs.
        '';
      };

      stateDir = lib.mkOption {
        type = lib.types.str;
        description = ''
          Per-VM state directory (`/var/lib/nixling/vms/<vm>`). Holds
          microvm.nix runner state, the `state/audio-state.json` file
          when `audio.enable`, and any per-VM scratch the framework
          owns. Path layout is currently hardcoded; see
          `nixling.site.stateDir`'s advisory-only note for the v0.2.0
          threading plan.
        '';
      };

      apiSocket = lib.mkOption {
        type = lib.types.str;
        description = ''
          microvm.nix runner API socket path
          (`<stateDir>/<vm>.sock`). The CLI uses it to query VM
          state (`crosvm control` / `cloud-hypervisor-api`) and to
          send a clean shutdown signal during `nixling down`.
        '';
      };

      gpuSocket = lib.mkOption {
        type = lib.types.str;
        description = ''
          crosvm-gpu sidecar control socket path
          (`<stateDir>/<vm>-gpu.sock`). Only meaningful when
          `graphics = true`.
        '';
      };

      tpmSocket = lib.mkOption {
        type = lib.types.str;
        description = ''
          swtpm vTPM socket path (`/run/swtpm/<vm>/sock`). Only
          meaningful when `tpm = true`. The framework's
          `nixling-<vm>-swtpm.service` opens this and the microvm.nix
          runner connects to it via `--tpm <socket>`.
        '';
      };

      audioStateFile = lib.mkOption {
        type = lib.types.str;
        description = ''
          Per-VM live audio-grant state file
          (`<stateDir>/state/audio-state.json`). Holds
          `{ "mic": "on"|"off", "speaker": "on"|"off" }`. Read by
          the host-side `nixling-<vm>-snd.service` sidecar (which
          re-routes vhost-device-sound's INPUT/OUTPUT links) and
          written atomically by `nixling audio …` subcommands.
        '';
      };

      audioService = lib.mkOption {
        type = lib.types.str;
        description = ''
          Name of the host-side per-VM audio sidecar systemd unit
          (`nixling-<vm>-snd.service`). The CLI restarts this unit
          on every audio-state change.
        '';
      };

      staticIp = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          The VM's static LAN IP. Derived from `(env, index)` for
          env-attached VMs and from `envMeta.netUplinkIp` for net
          VMs. Legacy VMs that set `nixling.vms.<vm>.staticIp`
          directly get that value passed through. Null when neither
          source applies (in which case the CLI cannot SSH and
          subcommands needing SSH refuse to run).
        '';
      };

      sshUser = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          Username for `nixling`-driven SSH into the VM. Mirrors
          `nixling.vms.<vm>.ssh.user`. Null is permitted (e.g. for
          headless net VMs that the CLI never SSH-attaches to);
          subcommands requiring SSH refuse to run when null.
        '';
      };
    };
  });
in

{
  options.nixling.manifest = lib.mkOption {
    type = lib.types.attrsOf manifestEntryType;
    readOnly = true;
    description = ''
      Per-VM metadata manifest, indexed by VM name. The contract a
      future Rust port of the `nixling` CLI consumes via
      `/run/current-system/sw/share/nixling/vms.json`.

      Computed by `nixos-modules/manifest.nix` from
      `config.nixling.vms.<name>` plus the per-env metadata produced
      by `network.nix`. Schema is documented in
      `docs/reference/manifest-schema.md` and formalised in
      `docs/reference/manifest-schema.json` (JSON Schema Draft 2020-12).

      Mark `readOnly` so consumers cannot accidentally override
      framework-computed fields. The whole point of this option is
      to be the single source of truth.

      No `default` is declared because the matching `config`
      assignment in this same module always sets the option
      unconditionally. nixpkgs' module-system counts `default` as a
      definition when computing the read-only conflict check
      (lib/modules.nix `evalOptionValue`: `defs' = [defaultDef] ++
      defs`), so a `default = { }` alongside the `config` assignment
      would trip `length defs' > 1` and abort eval the moment any
      caller forced the option — which is exactly what cli.nix's
      `vmLaunchScript` does for every graphics-enabled VM. The
      smoke-eval test never declared a graphics VM and so never
      exposed this; phase-6b's graphics-workstation example
      surfaced it. See plan.md "Spec corrections" entry #29.
    '';
  };

  options.nixling._manifestVersion = lib.mkOption {
    type = lib.types.ints.unsigned;
    default = 1;
    internal = true;
    description = ''
      Internal: the integer schema version stamped into
      `_manifest.manifestVersion` of the rendered JSON manifest.
      Bumped on every breaking schema change (field removed,
      renamed, retyped, or semantics changed). Additive changes
      (new optional fields) do not bump.

      Set in `manifest.nix`. Phase 5 introduced this option;
      consumers should not override.

      Version history:
        * 0 — pre-Phase-5 schema (W2-followup stub). Schema was
          undocumented and changed without bumps (e.g. the
          `isRouter`→`isNetVm` / `routerVm`→`netVm` rename in W2).
        * 1 — first documented, externally-stable version. Locks in
          the per-VM field set documented in
          `docs/reference/manifest-schema.{md,json}`.
    '';
  };

  options.nixling._manifestJsonPath = lib.mkOption {
    type = lib.types.str;
    default = "";
    internal = true;
    description = ''
      Internal: absolute store path to the rendered
      `vms.json` file. Consumed by `cli.nix` to bake the manifest
      path into the `nixling` shell wrapper.
    '';
  };

  options.nixling._manifestPkg = lib.mkOption {
    type = lib.types.package;
    internal = true;
    description = ''
      Internal: the derivation that ships
      `/share/nixling/vms.json`. Added to
      `environment.systemPackages` so the file ends up at
      `/run/current-system/sw/share/nixling/vms.json` at runtime
      (the path the future Rust CLI will look at without having to
      consult any other store path).
    '';
  };

  config = {
    nixling.manifest = computedManifest;
    nixling._manifestJsonPath = "${manifestPkg}/share/nixling/vms.json";
    nixling._manifestPkg = manifestPkg;
    environment.systemPackages = [ manifestPkg ];
  };
}
