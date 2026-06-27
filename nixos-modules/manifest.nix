# nixos-modules/manifest.nix — typed JSON manifest contract.
#
# Builds the per-VM JSON manifest that the (current bash, future Rust)
# d2b CLI consumes at runtime. The manifest is the stable contract
# between the Nix-evaluated framework state and the imperative CLI; it
# carries every piece of per-VM metadata the CLI needs at command
# dispatch time (socket paths, IPs, env membership, capability flags,
# SSH credentials, …).
#
# Why an externally-typed module instead of an ad-hoc let-binding in
# cli.nix
#
#   1. The JSON file at `/run/current-system/sw/share/d2b/vms.json`
#      is the integration surface for the Rust CLI port. It
#      must be documented and versioned. A typed `mkOption` gives us
#      a schema we can hand-walk into `docs/reference/manifest-schema.{md,json}`
#      and validate against in `tests/static.sh`.
#   2. The Nix module system catches schema regressions at eval time
#      if a future refactor accidentally produces a field of the wrong
#      type, evaluation fails immediately rather than silently shipping
#      a broken JSON file.
#   3. The CLI can consume `config.d2b.manifest` directly from
#      sibling modules (e.g. `cli.nix`'s per-VM exec launcher) with
#      type-checked attribute access, no second `lib.mapAttrs` of the
#      same data.
#
# The JSON file's top-level layout is
#
#   {
#     "_manifest": { "manifestVersion": <int> },
#     "<vmName>":  { name: ..., env: ..., apiSocket: ..., ... },
#     "<vmName>":  {... },
# ...
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
  d2bLib = import ./lib.nix { inherit lib pkgs; };
  inherit (d2bLib) subnetIp;

  envMeta = config.d2b._envMeta;
  enabledVms = lib.filterAttrs (_: vm: vm.enable) config.d2b.vms;
  obsCfg = config.d2b.observability;

  # `lib.attrNames` returns names sorted lexicographically, so the
  # env-index assignment is deterministic and stable across evals.
  envNames = lib.attrNames config.d2b.envs;
  envIndexMap = lib.listToAttrs (
    lib.imap0 (i: name: { inherit name; value = i; }) envNames
  );

  netVmOfEnv = envName:
    let n = config.d2b.envs.${envName}.netName or "sys-${envName}-net";
    in n;

  envOfNetVm = name:
    lib.findFirst
      (e: netVmOfEnv e == name)
      null
      (lib.attrNames config.d2b.envs);

  vmMeta = name: vm:
    let
      env = vm.env;
      asNetVmForEnv = envOfNetVm name;
      envName = if env != null then env else asNetVmForEnv;
      m = if env != null && envMeta ? ${env} then envMeta.${env} else null;
      runtime = d2bLib.vmRuntimeMetadata name vm;
      isNixosRuntime = runtime.kind == "nixos";
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
        if isNixosRuntime && m != null then m.hostUplinkIp
        else null;
      stateRoot = "${config.d2b.store.stateDir}/${name}";
      envIndex =
        if envName != null && envIndexMap ? ${envName}
        then envIndexMap.${envName}
        else null;
      baseVsockCid =
        if isNixosRuntime then d2bLib.guestControlVsockCid {
          inherit name envIndex;
          index = vm.index;
          isNetVm = asNetVmForEnv != null;
          isObservabilityVm = obsCfg.enable && name == obsCfg.vmName;
        } else null;
      baseVsockHostSocket =
        if isNixosRuntime then d2bLib.guestControlVsockHostSocket stateRoot else null;
    in
    {
      inherit name;
      runtime = runtime;
      lifecycle = {
        gracefulShutdown = {
          enable = vm.lifecycle.gracefulShutdown.enable;
          timeoutSeconds = vm.lifecycle.gracefulShutdown.timeoutSeconds;
        };
        liveActivation = {
          timeoutSeconds = vm.lifecycle.liveActivation.timeoutSeconds;
        };
      };
      graphics = isNixosRuntime && vm.graphics.enable;
      tpm = isNixosRuntime && vm.tpm.enable;
      usbipYubikey = isNixosRuntime && vm.usbip.yubikey;
      audio = isNixosRuntime && vm.audio.enable;
      tap = derivedTap;
      bridge = derivedBridge;
      env = envName;
      isNetVm = asNetVmForEnv != null;
      netVm = if env != null then netVmOfEnv env else null;
      usbipdHostIp = usbipdHostIp;
      stateDir = stateRoot;
      apiSocket = if isNixosRuntime then "${stateRoot}/${name}.sock" else null;
      gpuSocket = if isNixosRuntime then "${stateRoot}/${name}-gpu.sock" else null;
      tpmSocket = if isNixosRuntime then "/run/d2b/vms/${name}/tpm.sock" else null;
      # State file under root-owned non-group-writable subdir.
      audioStateFile =
        if isNixosRuntime then "${stateRoot}/state/audio-state.json" else null;
      audioService = if isNixosRuntime then "d2b-${name}-snd.service" else null;
      observability = {
        enabled = isNixosRuntime && vm.observability.enable;
        vsockCid = baseVsockCid;
        vsockHostSocket = baseVsockHostSocket;
        agentSocket = if isNixosRuntime then "/run/d2b/otlp.sock" else null;
      };
      shell =
        if isNixosRuntime then {
          enabled = vm.guest.shell.enable;
          defaultName = vm.guest.shell.defaultName;
          maxSessions = vm.guest.shell.maxSessions;
          maxAttached = vm.guest.shell.maxAttached;
        } else null;
      staticIp =
        if derivedIp != null then derivedIp
        else vm.staticIp;
      sshUser = if isNixosRuntime then vm.ssh.user else null;
      # `sshKeyPath` is intentionally NOT part of the public manifest.
      # The manifest ships to
      # `/run/current-system/sw/share/d2b/vms.json` which is
      # world-readable; exposing a per-VM private-key path there
      # leaks the location of secret material to every local user.
      # The CLI resolves the private-key path locally from
      # `config.d2b.site.keysDir` (or `vm.ssh.keyPath` when the
      # consumer overrides it) at Nix-eval time and bakes the
      # per-VM mapping into the shell wrapper. Consumers
      # reimplementing the CLI should mirror that — read
      # `d2b.site.keysDir` from their own privileged config
      # access, not from this world-readable file. The PUBLIC key
      # is fine to expose; if a future use case warrants it, add
      # `sshPubKeyPath` here.
    };

    manifestShellType = lib.types.submodule {
      freeformType = null;
      options = {
        enabled = lib.mkOption {
          type = lib.types.bool;
          description = ''
            True iff `d2b.vms.<name>.guest.shell.enable` is set on a runtime
            provider that supports persistent guest shells.
          '';
        };

        defaultName = lib.mkOption {
          type = lib.types.strMatching "^[A-Za-z0-9_][A-Za-z0-9._-]{0,63}$";
          description = "Default persistent shell session name.";
        };

        maxSessions = lib.mkOption {
          type = lib.types.ints.between 1 256;
          description = "Maximum persistent shell sessions for this VM.";
        };

        maxAttached = lib.mkOption {
          type = lib.types.ints.between 1 64;
          description = "Maximum concurrently attached persistent shell clients for this VM.";
        };
      };
    };

  computedManifest = lib.mapAttrs vmMeta enabledVms;

  # Top-level JSON shape: per-VM entries side-by-side with the
  # reserved `_manifest` schema-version sentinel and the observability
  # capability sentinel. The CLI's jq filters all use `--arg n` +
  # `.[$n]` lookups, so reserved-key additions do not affect per-VM
  # iteration patterns.
  manifestJson = computedManifest // {
    _manifest = {
      manifestVersion = config.d2b._manifestVersion;
    };
    _observability = {
      enabled = obsCfg.enable;
      vmName = obsCfg.vmName;
      obsVsockCid = 1000;
      obsVsockHostSocket =
        d2bLib.guestControlVsockHostSocket "${config.d2b.store.stateDir}/${obsCfg.vmName}";
      signozUrl = "http://${obsCfg.signoz.listenAddress}:${toString obsCfg.signoz.listenPort}";
      signozOtlpGrpcPort = obsCfg.signoz.otlpGrpcPort;
      signozOtlpHttpPort = obsCfg.signoz.otlpHttpPort;
    };
  };

  manifestPkg = pkgs.writeTextFile {
    name = "d2b-vms-manifest";
    text = builtins.toJSON manifestJson;
    destination = "/share/d2b/vms.json";
  };

  runtimeProviderType = lib.types.submodule {
    freeformType = null;
    options = {
      id = lib.mkOption {
        type = lib.types.enum [ "local-cloud-hypervisor" "local-qemu-media" ];
        description = "Stable local runtime provider identifier.";
      };

      type = lib.mkOption {
        type = lib.types.enum [ "local" ];
        description = "Provider locality class. `local` means host-local VMM/provider state.";
      };

      driver = lib.mkOption {
        type = lib.types.enum [ "cloud-hypervisor" "qemu" ];
        description = "Provider driver family.";
      };
    };
  };

  runtimeCapabilitiesType = lib.types.submodule {
    freeformType = null;
    options = {
      lifecycle = lib.mkOption { type = lib.types.bool; };
      display = lib.mkOption { type = lib.types.bool; };
      usbHotplug = lib.mkOption { type = lib.types.bool; };
      guestControl = lib.mkOption { type = lib.types.bool; };
      exec = lib.mkOption { type = lib.types.bool; };
      configSync = lib.mkOption { type = lib.types.bool; };
      ssh = lib.mkOption { type = lib.types.bool; };
      storeSync = lib.mkOption { type = lib.types.bool; };
      keys = lib.mkOption { type = lib.types.bool; };
      inGuestObservability = lib.mkOption { type = lib.types.bool; };
    };
  };

  runtimeOperationCapabilitiesType = lib.types.submodule {
    freeformType = null;
    options = {
      lifecycle = lib.mkOption {
        type = lib.types.submodule {
          freeformType = null;
          options = {
            start = lib.mkOption { type = lib.types.bool; };
            stop = lib.mkOption { type = lib.types.bool; };
            restart = lib.mkOption { type = lib.types.bool; };
            switch = lib.mkOption { type = lib.types.bool; };
            hostPrepare = lib.mkOption { type = lib.types.bool; };
          };
        };
      };
      media = lib.mkOption {
        type = lib.types.submodule {
          freeformType = null;
          options = {
            usbHotplug = lib.mkOption { type = lib.types.bool; };
            removableMedia = lib.mkOption { type = lib.types.bool; };
            qemuMedia = lib.mkOption { type = lib.types.bool; };
          };
        };
      };
      display = lib.mkOption {
        type = lib.types.submodule {
          freeformType = null;
          options = {
            display = lib.mkOption { type = lib.types.bool; };
            graphics = lib.mkOption { type = lib.types.bool; };
            video = lib.mkOption { type = lib.types.bool; };
            waylandProxy = lib.mkOption { type = lib.types.bool; };
          };
        };
      };
      guest = lib.mkOption {
        type = lib.types.submodule {
          freeformType = null;
          options = {
            guestControl = lib.mkOption { type = lib.types.bool; };
            exec = lib.mkOption { type = lib.types.bool; };
            shell = lib.mkOption { type = lib.types.bool; };
            configSync = lib.mkOption { type = lib.types.bool; };
            ssh = lib.mkOption { type = lib.types.bool; };
            keys = lib.mkOption { type = lib.types.bool; };
            inGuestObservability = lib.mkOption { type = lib.types.bool; };
          };
        };
      };
      storage = lib.mkOption {
        type = lib.types.submodule {
          freeformType = null;
          options = {
            storeSync = lib.mkOption { type = lib.types.bool; };
            virtiofs = lib.mkOption { type = lib.types.bool; };
            volumes = lib.mkOption { type = lib.types.bool; };
          };
        };
      };
    };
  };

  runtimeServiceSummaryType = lib.types.submodule {
    freeformType = null;
    options = {
      id = lib.mkOption { type = lib.types.str; };
      role = lib.mkOption {
        type = lib.types.enum [
          "host"
          "hypervisor"
          "storage"
          "tpm"
          "display"
          "audio"
          "video"
          "network"
          "guest-control"
          "usb"
          "observability"
        ];
      };
      optional = lib.mkOption {
        type = lib.types.bool;
        default = false;
      };
    };
  };

  runtimeMetadataType = lib.types.submodule {
    freeformType = null;
    options = {
      kind = lib.mkOption {
        type = lib.types.enum [ "nixos" "qemu-media" ];
        description = "VM runtime family.";
      };

      provider = lib.mkOption {
        type = runtimeProviderType;
        description = "Local provider selected for this runtime kind.";
      };

      capabilities = lib.mkOption {
        type = runtimeCapabilitiesType;
        description = ''
          Provider support matrix. These booleans describe whether the runtime
          provider supports a capability, not whether a per-VM option currently
          enables that feature.
        '';
      };

      operationCapabilities = lib.mkOption {
        type = runtimeOperationCapabilitiesType;
        description = "Positive operation support grouped by public feature axis.";
      };

      autostartPolicy = lib.mkOption {
        type = lib.types.enum [ "unknown" "host-boot-eligible" "manual-only" "disabled" ];
        default = "unknown";
        description = "Runtime-level autostart policy.";
      };

      services = lib.mkOption {
        type = lib.types.listOf runtimeServiceSummaryType;
        default = [ ];
        description = "Provider-neutral runtime service summaries.";
      };
    };
  };

  manifestGracefulShutdownType = lib.types.submodule {
    freeformType = null;
    options = {
      enable = lib.mkOption {
        type = lib.types.bool;
        description = ''
          True iff d2bd should attempt provider-aware graceful guest
          shutdown for this VM before falling back to forced VMM cleanup.
        '';
      };

      timeoutSeconds = lib.mkOption {
        type = lib.types.nullOr lib.types.int;
        description = ''
          Optional per-VM graceful shutdown timeout override, in seconds. Null
          means the daemon default from daemon-config.json applies.
        '';
      };
    };
  };

  manifestLifecycleType = lib.types.submodule {
    freeformType = null;
    options = {
      gracefulShutdown = lib.mkOption {
        type = manifestGracefulShutdownType;
        description = "Per-VM graceful guest shutdown policy.";
      };

      liveActivation = lib.mkOption {
        type = lib.types.submodule {
          freeformType = null;
          options = {
            timeoutSeconds = lib.mkOption {
              type = lib.types.nullOr lib.types.int;
              description = ''
                Optional per-VM live activation timeout override, in seconds.
                Null means the daemon default from daemon-config.json applies.
              '';
            };
          };
        };
        description = "Per-VM live in-guest activation policy.";
      };
    };
  };

  manifestObservabilityType = lib.types.submodule {
    freeformType = null;
    options = {
      enabled = lib.mkOption {
        type = lib.types.bool;
        description = ''
          True iff `d2b.vms.<name>.observability.enable` is set.
        '';
      };
      vsockCid = lib.mkOption {
        type = lib.types.nullOr lib.types.ints.unsigned;
        description = ''
          Deterministic base Cloud Hypervisor vsock CID for nixos/Cloud
          Hypervisor VMs. Env-backed VMs use `100 + envIndex * 1000 + slot`,
          where slot 1 is reserved for the env net VM and workload VMs use
          `d2b.vms.<vm>.index`. Null for providers that do not expose
          d2b guest-control or in-guest observability.
        '';
      };

      vsockHostSocket = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          Host-side Unix socket backing this VM's Cloud Hypervisor vsock
          device. Null for providers without d2b guest-control or
          in-guest observability.
        '';
      };

      agentSocket = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          In-guest Unix socket the observability guest agent listens on for
          local OTLP traffic. Null for providers without in-guest
          observability.
        '';
      };
    };
  };

  # Per-VM submodule type, matching docs/reference/manifest-schema.json. Every
  # field is declared with a concrete type; the module system fails the
  # eval if the computed assignment from `vmMeta` ever drifts (e.g. a
  # refactor returns `null` for a field declared `str`).
  manifestEntryType = lib.types.submodule ({ name, ... }: {
    options = {
      name = lib.mkOption {
        type = lib.types.str;
        description = "VM name (attribute key in d2b.vms.<name>).";
      };

      runtime = lib.mkOption {
        type = runtimeMetadataType;
        description = ''
          Runtime/provider metadata and provider capability summary. This is
          the provider-neutral dispatch surface for daemon lifecycle/status
          integration; provider-specific runner details stay in private bundle
          artifacts.
        '';
      };

      lifecycle = lib.mkOption {
        type = manifestLifecycleType;
        description = ''
          Per-VM lifecycle policy consumed by d2bd. v7 currently contains
          provider-aware graceful guest shutdown enablement and optional
          timeout override metadata.
        '';
      };

      graphics = lib.mkOption {
        type = lib.types.bool;
        description = ''
          True iff `d2b.vms.<name>.graphics.enable` is set. The CLI
          uses this to pick the launch path.
        '';
      };

      tpm = lib.mkOption {
        type = lib.types.bool;
        description = "True iff `d2b.vms.<name>.tpm.enable` is set.";
      };

      usbipYubikey = lib.mkOption {
        type = lib.types.bool;
        description = ''
          True iff `d2b.vms.<name>.usbip.yubikey` is set. The CLI's
          `d2b usb` subcommand refuses to run for VMs where this is
          false.
        '';
      };

      audio = lib.mkOption {
        type = lib.types.bool;
        description = ''
          True iff `d2b.vms.<name>.audio.enable` is set. Live audio
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
          (net VMs first) and to skip net VMs in `d2b up <env>`
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
          (`sys-<env>-usbipd`/`proxy` broker runner). The `d2b
          usb` subcommand passes this as `-r <ip>` to `usbip attach`.
          Null for net VMs and legacy VMs.
        '';
      };

      stateDir = lib.mkOption {
        type = lib.types.str;
        description = ''
          Per-VM state directory (`/var/lib/d2b/vms/<vm>`). Holds
          microvm.nix runner state, the `state/audio-state.json` file
          when `audio.enable`, and any per-VM scratch the framework
          owns. Path layout is currently hardcoded; see
          `d2b.site.stateDir`'s advisory-only note for the v0.2.0
          threading plan.
        '';
      };

      apiSocket = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          Cloud Hypervisor runner API socket path (`<stateDir>/<vm>.sock`).
          Null for runtime providers that do not expose a Cloud Hypervisor API
          socket.
        '';
      };

      gpuSocket = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          crosvm-gpu sidecar control socket path
          (`<stateDir>/<vm>-gpu.sock`). Only meaningful when
          `graphics = true`; null when the runtime provider has no d2b GPU
          sidecar socket.
        '';
      };

      tpmSocket = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          swtpm vTPM socket path (`/run/d2b/vms/<vm>/tpm.sock`).
          Only meaningful when `tpm = true`. The framework's
          long-lived swtpm sidecar (spawned by the broker) creates
          this socket; cloud-hypervisor connects to it via
          `--tpm <socket>`. Lives under the per-VM runtime dir so
          the existing default ACL grants every per-VM ephemeral
          UID (including cloud-hypervisor) rw on it. Null for providers without
          d2b-managed TPM state.
        '';
      };

      audioStateFile = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          Per-VM live audio-grant state file
          (`<stateDir>/state/audio-state.json`). Holds
          `{ "mic": "on"|"off", "speaker": "on"|"off" }`. Read by
          the host-side `d2b-<vm>-snd.service` sidecar (which
          re-routes vhost-device-sound's INPUT/OUTPUT links) and
          written atomically by `d2b audio …` subcommands. Null for
          providers without the d2b audio sidecar.
        '';
      };

      audioService = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          Name of the host-side per-VM audio sidecar systemd unit
          (`d2b-<vm>-snd.service`). The CLI restarts this unit
          on every audio-state change. Null for providers without the d2b
          audio sidecar.
        '';
      };

      staticIp = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          The VM's static LAN IP. Derived from `(env, index)` for
          env-attached VMs and from `envMeta.netUplinkIp` for net
          VMs. Legacy VMs that set `d2b.vms.<vm>.staticIp`
          directly get that value passed through. Null when neither
          source applies (in which case the CLI cannot SSH and
          subcommands needing SSH refuse to run).
        '';
      };

      sshUser = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        description = ''
          Username for `d2b`-driven SSH into the VM. Mirrors
          `d2b.vms.<vm>.ssh.user`. Null is permitted (e.g. for
          headless net VMs that the CLI never SSH-attaches to);
          subcommands requiring SSH refuse to run when null.
        '';
      };

      observability = lib.mkOption {
        type = manifestObservabilityType;
        description = ''
          Per-VM observability transport metadata. Always emitted so the
          observability track can rely on the field existing even before
          the sidecars land.
        '';
      };

      shell = lib.mkOption {
        type = lib.types.nullOr manifestShellType;
        description = ''
          Persistent guest shell policy metadata for providers that support the
          authenticated guest-control terminal substrate. Null for runtime
          providers without d2b guest-control.
        '';
      };
    };
  });
in

{
  options.d2b.manifest = lib.mkOption {
    type = lib.types.attrsOf manifestEntryType;
    readOnly = true;
    description = ''
      Per-VM metadata manifest, indexed by VM name. The contract a
      future Rust port of the `d2b` CLI consumes via
      `/run/current-system/sw/share/d2b/vms.json`.

      Computed by `nixos-modules/manifest.nix` from
      `config.d2b.vms.<name>` plus the per-env metadata produced
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
      exposed this; the graphics-workstation example surfaced it.
    '';
  };

  options.d2b._manifestVersion = lib.mkOption {
    type = lib.types.ints.unsigned;
    default = 7;
    internal = true;
    description = ''
      Internal: the integer schema version stamped into
      `_manifest.manifestVersion` of the rendered JSON manifest.
      Bumped on every breaking schema change (field removed,
      renamed, retyped, or semantics changed). Additive changes
      (new optional fields) do not bump.

      Set in `manifest.nix`; consumers should not override.

      Version history:
        * 0 — pre-documented schema. Schema was
          undocumented and changed without bumps (e.g. the
          `isRouter`→`isNetVm` / `routerVm`→`netVm` rename).
        * 1 — first documented, externally-stable version. Locks in
          the baseline per-VM field set documented in
          `docs/reference/manifest-schema.{md,json}`.
        * 2 — observability schema expansion. Adds the always-emitted
          per-VM `observability` block and the top-level
          `_observability` sentinel.
        * 3 — daemon-only end-state break. Drops per-VM systemd-unit
          reference fields that become meaningless once supervisor
          mode is retired and the daemon owns every per-VM lifecycle
          transition.
        * 4 — base Cloud Hypervisor vsock semantics. Keeps the v3
          shape, but defines per-VM `observability.vsockCid` and
          `observability.vsockHostSocket` as the host-owned base
          vsock device used by observability today and guest control in
          later waves. Pinned by
          `d2b_core::manifest_v04::MANIFEST_VERSION_CURRENT`; the
          broker / daemon refuse any other value with a
          `manifest-version-mismatch` typed error (no legacy
          compatibility window).
        * 5 — combines two independent contract changes that each
          landed as a `4` on separate branches: the base Cloud
          Hypervisor vsock semantics above, and the native SigNoz
          observability backend, which replaces the top-level
          `_observability` Grafana / Cloud Hypervisor exporter metadata
          (`grafanaUrl`, `chExporter`) with SigNoz UI and
          collector-ingress metadata (`signozUrl`, `signozOtlpGrpcPort`,
          `signozOtlpHttpPort`). The vsock transport contract is
          unchanged. Pinned by
          `d2b_core::manifest_v04::MANIFEST_VERSION_CURRENT`.
        * 6 — adds per-VM runtime/provider metadata and provider
          capability summaries, and makes provider-specific socket/vsock
          fields nullable so qemu-media entries do not fabricate Cloud
          Hypervisor, guest-control, SSH, store-sync, key, or
          in-guest-observability artifacts.
        * 7 — adds per-VM lifecycle.gracefulShutdown metadata so d2bd
          can apply VM-specific graceful guest-shutdown policy while
          preserving old-manifest compatibility during the v6→v7 rollout.
    '';
  };

  options.d2b._manifestJsonPath = lib.mkOption {
    type = lib.types.str;
    default = "";
    internal = true;
    description = ''
      Internal: absolute store path to the rendered
      `vms.json` file. Consumed by `cli.nix` to bake the manifest
      path into the `d2b` shell wrapper.
    '';
  };

  options.d2b._manifestPkg = lib.mkOption {
    type = lib.types.package;
    internal = true;
    description = ''
      Internal: the derivation that ships
      `/share/d2b/vms.json`. Added to
      `environment.systemPackages` so the file ends up at
      `/run/current-system/sw/share/d2b/vms.json` at runtime
      (the path the future Rust CLI will look at without having to
      consult any other store path).
    '';
  };

  config = {
    d2b.manifest = computedManifest;
    d2b._manifestJsonPath = "${manifestPkg}/share/d2b/vms.json";
    d2b._manifestPkg = manifestPkg;
    environment.systemPackages = [ manifestPkg ];
  };
}
