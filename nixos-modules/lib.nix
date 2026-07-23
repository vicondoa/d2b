# Shared pure helpers for the d2b framework. Imported as a
# function (`import ./lib.nix { inherit lib; }`) by network.nix and
# host.nix so they share the same MAC/IP derivation rules.
#
# Pass `pkgs` as well (`import ./lib.nix { inherit lib pkgs; }`) to
# get `d2bReadAudioState`, a Nix-store shell fragment that both
# audio.nix and cli.nix source for fail-closed audio-state reads.
{ lib, pkgs ? null }:

let
  hex2 = i:
    let s = lib.toHexString i;
    in if lib.stringLength s == 1 then "0${s}" else s;

  # d2b_read_audio_state <vm>
  # ------------------------------------------------------------
  # Fail-closed reader for /var/lib/d2b/<vm>/audio-state.json.
  # Output (one line on stdout): "mic=<on|off> speaker=<on|off>".
  # NEVER exits non-zero — callers (extraArgsScript, d2b CLI)
  # cannot handle a non-zero exit mid-flow.
  #
  # Returns "mic=off speaker=off" for EVERY error case
  #   • file missing
  #   • file present but unreadable (permissions)
  #   • file present but not valid JSON
  #   • field absent
  #   • field present but value is not the exact string "on"
  #     (e.g. boolean true, number 1, string "true", string "ON")
  #   • jq not on PATH (path is Nix-store–hardcoded below)
  #
  # The jq path is baked in at Nix eval time so the function works
  # in both audio.nix's extraArgsScript (minimal $PATH) and the
  # d2b shell application (jq also in runtimeInputs, harmless).
  d2bReadAudioState =
    if pkgs == null then null
    else
      pkgs.writeText "d2b-read-audio-state.sh" ''
        d2b_read_audio_state() {
          local _nas_vm="$1" _nas_f _nas_mic=off _nas_spk=off _nas_raw
          local _nas_canonical _nas_expected _nas_stat
          # State file lives under the root-owned state/ subdir.
          # VM state dir moved under vms/<vm>/.
          _nas_f="/var/lib/d2b/vms/$_nas_vm/state/audio-state.json"
          _nas_expected="/var/lib/d2b/vms/$_nas_vm/state/audio-state.json"
          # Canonicalize: fail closed if path doesn't resolve or is a symlink
          # pointing outside the expected location.
          _nas_canonical=$(realpath -e "$_nas_f" 2>/dev/null) \
            || { printf 'mic=off speaker=off\n'; return 0; }
          [ "$_nas_canonical" = "$_nas_expected" ] \
            || { printf 'mic=off speaker=off\n'; return 0; }
          # Verify ownership and mode: must be root:d2b 640.
          _nas_stat=$(stat -c '%U %G %a' "$_nas_canonical" 2>/dev/null) \
            || { printf 'mic=off speaker=off\n'; return 0; }
          [ "$_nas_stat" = "root d2b 640" ] \
            || { printf 'mic=off speaker=off\n'; return 0; }
          if [ -r "$_nas_canonical" ]; then
            if _nas_raw=$(${pkgs.jq}/bin/jq -re '.mic' "$_nas_canonical" 2>/dev/null) \
               && [ "$_nas_raw" = "on" ]; then
              _nas_mic=on
            fi
            if _nas_raw=$(${pkgs.jq}/bin/jq -re '.speaker' "$_nas_canonical" 2>/dev/null) \
               && [ "$_nas_raw" = "on" ]; then
              _nas_spk=on
            fi
          else
            # software-r2-1: file exists but is unreadable by the calling user
            # (e.g. an interactive operator or d2b-gpu-<vm> before ACLs are
            # applied). Fail closed so the sidecar never gets audio access on
            # a permission error.
            printf 'd2b: audio-state unreadable for %s (permission denied) — failing closed\n' "$_nas_vm" >&2
          fi
          printf 'mic=%s speaker=%s\n' "$_nas_mic" "$_nas_spk"
        }
      '';
in
rec {
  inherit hex2;
  inherit d2bReadAudioState;

  cleanRustPackagesSource = packagesPath:
    lib.cleanSourceWith {
      src = packagesPath;
      filter = path: type:
        let rel = lib.removePrefix (toString packagesPath + "/") (toString path);
        in !(
          (type == "directory" && baseNameOf path == "target")
          || lib.hasInfix ".cargo/registry" rel
        );
    };

  vmRuntimeKind = vm: vm.runtime.kind or "nixos";
  isNixosVm = vm: vmRuntimeKind vm == "nixos";
  isQemuMediaVm = vm: vmRuntimeKind vm == "qemu-media";

  enabledVms = vms: lib.filterAttrs (_: vm: vm.enable) vms;
  normalNixosVms = vms: lib.filterAttrs (_: vm: vm.enable && isNixosVm vm) vms;
  qemuMediaVms = vms: lib.filterAttrs (_: vm: vm.enable && isQemuMediaVm vm) vms;

  localRuntimeProvider = { id, driver }: {
    inherit id driver;
    type = "local";
  };

  nixosRuntimeProvider = localRuntimeProvider {
    id = "local-cloud-hypervisor";
    driver = "cloud-hypervisor";
  };

  qemuMediaRuntimeProvider = localRuntimeProvider {
    id = "local-qemu-media";
    driver = "qemu";
  };

  mkServiceCapability =
    { supported
    , nodeId ? null
    , runnerRole ? null
    , driver ? null
    , readiness ? null
    , contract ? null
    , transport ? null
    , unitStrategy ? null
    }:
    {
      inherit supported;
    }
    // lib.optionalAttrs (nodeId != null) { inherit nodeId; }
    // lib.optionalAttrs (runnerRole != null) { inherit runnerRole; }
    // lib.optionalAttrs (driver != null) { inherit driver; }
    // lib.optionalAttrs (readiness != null) { inherit readiness; }
    // lib.optionalAttrs (contract != null) { inherit contract; }
    // lib.optionalAttrs (transport != null) { inherit transport; }
    // lib.optionalAttrs (unitStrategy != null) { inherit unitStrategy; };

  mkRuntimeCapabilities =
    { legacy }:
    legacy;

  runtimeServiceSummary = { id, role, optional ? false }: {
    inherit id role optional;
  };

  nixosRuntimeCapabilities = mkRuntimeCapabilities {
    legacy = {
      lifecycle = true;
      display = true;
      usbHotplug = true;
      guestControl = true;
      exec = true;
      configSync = true;
      ssh = true;
      storeSync = true;
      keys = true;
      inGuestObservability = true;
    };
  };

  nixosRuntimeOperationCapabilities = {
    lifecycle = {
      start = true;
      stop = true;
      restart = true;
      switch = true;
      hostPrepare = true;
    };
    media = {
      usbHotplug = true;
      removableMedia = false;
      qemuMedia = false;
    };
    display = {
      display = true;
      graphics = true;
      video = true;
      waylandProxy = true;
    };
    guest = {
      guestControl = true;
      exec = true;
      shell = true;
      configSync = true;
      ssh = true;
      keys = true;
      inGuestObservability = true;
    };
    storage = {
      storeSync = true;
      virtiofs = true;
      volumes = true;
    };
  };

  qemuMediaRuntimeCapabilities = mkRuntimeCapabilities {
    legacy = {
      lifecycle = true;
      display = true;
      usbHotplug = true;
      guestControl = false;
      exec = false;
      configSync = false;
      ssh = false;
      storeSync = false;
      keys = false;
      inGuestObservability = false;
    };
  };

  qemuMediaRuntimeOperationCapabilities = {
    lifecycle = {
      start = true;
      stop = true;
      restart = true;
      switch = false;
      hostPrepare = true;
    };
    media = {
      usbHotplug = true;
      removableMedia = true;
      qemuMedia = true;
    };
    display = {
      display = true;
      graphics = false;
      video = false;
      waylandProxy = false;
    };
    guest = {
      guestControl = false;
      exec = false;
      shell = false;
      configSync = false;
      ssh = false;
      keys = false;
      inGuestObservability = false;
    };
    storage = {
      storeSync = false;
      virtiofs = false;
      volumes = false;
    };
  };

  nixosHypervisorService = mkServiceCapability {
    supported = true;
    nodeId = "cloud-hypervisor";
    runnerRole = "cloud-hypervisor-runner";
    driver = "cloud-hypervisor";
    readiness = "api-socket";
    contract = "spawn-runner";
    unitStrategy = "microvm-or-graphics-sidecar";
  };

  qemuMediaHypervisorService = mkServiceCapability {
    supported = true;
    nodeId = "qemu-media";
    runnerRole = "qemu-media-runner";
    driver = "qemu";
    readiness = "qmp-socket";
    contract = "spawn-runner";
    unitStrategy = "daemon-supervised-runner";
  };

  runtimeHypervisorService = kind: (runtimeProviderCatalog.${kind}
    or (throw "d2b: unsupported runtime kind '${kind}'"))._hypervisorService;

  runtimeProviderCatalog = {
    nixos = {
      kind = "nixos";
      provider = nixosRuntimeProvider;
      capabilities = nixosRuntimeCapabilities;
      operationCapabilities = nixosRuntimeOperationCapabilities;
      autostartPolicy = "host-boot-eligible";
      services = [
        (runtimeServiceSummary { id = "host-reconcile"; role = "host"; })
        (runtimeServiceSummary { id = "store-virtiofs-preflight"; role = "storage"; })
        (runtimeServiceSummary { id = "virtiofsd"; role = "storage"; })
        (runtimeServiceSummary { id = "cloud-hypervisor"; role = "hypervisor"; })
        (runtimeServiceSummary { id = "guest-control-health"; role = "guest-control"; })
        (runtimeServiceSummary { id = "swtpm"; role = "tpm"; optional = true; })
        (runtimeServiceSummary { id = "gpu"; role = "display"; optional = true; })
        (runtimeServiceSummary { id = "audio"; role = "audio"; optional = true; })
        (runtimeServiceSummary { id = "video"; role = "video"; optional = true; })
        (runtimeServiceSummary { id = "usbip"; role = "usb"; optional = true; })
      ];
      _hypervisorService = nixosHypervisorService;
    };
    qemu-media = {
      kind = "qemu-media";
      provider = qemuMediaRuntimeProvider;
      capabilities = qemuMediaRuntimeCapabilities;
      operationCapabilities = qemuMediaRuntimeOperationCapabilities;
      autostartPolicy = "manual-only";
      services = [
        (runtimeServiceSummary { id = "host-reconcile"; role = "host"; })
        (runtimeServiceSummary { id = "qemu-media"; role = "hypervisor"; })
        (runtimeServiceSummary { id = "usbip"; role = "usb"; optional = true; })
      ];
      _hypervisorService = qemuMediaHypervisorService;
    };
  };

  vmRuntimeMetadata = _name: vm:
    let
      kind = vmRuntimeKind vm;
      runtime = runtimeProviderCatalog.${kind}
        or (throw "d2b: unsupported runtime kind '${kind}'");
    in builtins.removeAttrs runtime [ "_hypervisorService" ];

  # Shared helper extracted from minijail-profiles.nix and
  # host-users.nix to eliminate the 4-line duplicate that was a
  # drift-risk for broker/ownership-matrix UID agreement. If the hash
  # algorithm or offset changes here, both consumers see the same UID,
  # preventing the ownership-matrix bug from silently returning.
  #
  # Maps a principal name (e.g. "d2b-work-aad-swtpm") to a
  # stable deterministic 24-bit UID in the range 50000..16827215.
  # `principal == "root"` short-circuits to UID 0 for the broker's
  # root-carve-out paths (ADR 0003).
  #
  # Birthday-bound collision risk: 50% at ~4096 principals,
  # 1% at ~410. Typical workstation deployments stay under 400
  # principals (≤100 VMs × 4 roles). For larger deployments,
  # extend the hash to 8 hex chars (32 bits, ~65k birthday-bound).
  # Eval-time collision detection lives in minijail-profiles.nix.
  stablePrincipalId = principal:
    if principal == "root" then 0
    else 50000 + lib.fromHexString (builtins.substring 0 6 (builtins.hashString "sha256" principal));

  # Stable virtio-blk serial for a microvm.volumes entry. Cloud Hypervisor
  # emits this into the block device, while vm-guest-base.nix mounts by the
  # corresponding /dev/disk/by-id/virtio-<serial> path.
  volumeSerial = volume:
    if (volume.serial or null) != null then volume.serial else (
      let
        base = baseNameOf (toString volume.image);
        withoutImg = lib.removeSuffix ".img" base;
        sanitized = lib.replaceStrings [ "." "_" "/" " " "," "=" ] [ "-" "-" "-" "-" "-" "-" ] withoutImg;
      in
      if sanitized == "" then "disk" else sanitized
    );

  volumeHostPath = stateDir: vmName: volume:
    let image = toString volume.image; in
    if lib.hasPrefix "/" image then image else "${toString stateDir}/${vmName}/${image}";

  volumeDiskInitEligible = volume:
    !(lib.hasPrefix "/" (toString volume.image))
    && (toString (volume.imageType or "raw")) == "raw"
    && (toString (volume.fsType or "ext4")) == "ext4";

  volumeSizeBytes = volume: (volume.size or 1024) * 1024 * 1024;

  volumeFileSystem = volume: {
    device = "/dev/disk/by-id/virtio-${volumeSerial volume}";
    fsType = volume.fsType or "ext4";
    options = [ "x-systemd.after=systemd-modules-load.service" ]
      ++ lib.optional (volume.readOnly or false) "ro";
    neededForBoot = true;
  };

  guestControlVsockPort = 14318;
  observabilityOtlpVsockPort = 14317;
  # AF_VSOCK port used by the d2b security-key CTAPHID relay frontend.
  # The guest sk-frontend connects on this port to the host broker.
  securityKeyVsockPort = 14320;
  observabilityStackVsockCid = 1000;

  # Deterministic per-VM Cloud Hypervisor vsock CID. Env-backed VMs
  # reserve slot 1 for the env net VM and use d2b.vms.<vm>.index
  # for workloads (10..250). The stride intentionally exceeds the
  # maximum workload index so adjacent envs cannot collide.
  guestControlVsockCid = { name, envIndex ? null, index ? null, isNetVm ? false, isObservabilityVm ? false }:
    if isObservabilityVm then observabilityStackVsockCid
    else if envIndex != null then
      let slot = if isNetVm then 1 else index; in
      100 + (envIndex * 1000) + slot
    else
      4096 + lib.fromHexString (builtins.substring 0 6 (builtins.hashString "md5" name));

  guestControlVsockHostSocket = stateRoot: "${stateRoot}/vsock.sock";

  volumeSerialIssues = volumes:
    let
      serials = map volumeSerial volumes;
    in {
      duplicates = lib.unique (lib.filter
        (serial: lib.count (candidate: candidate == serial) serials > 1)
        serials);
      reserved = lib.filter (serial: serial == "rootfs") serials;
      tooLong = lib.filter (serial: lib.stringLength serial > 20) serials;
      unsafe = lib.filter
        (serial: builtins.match "^[A-Za-z0-9][A-Za-z0-9-]{0,19}$" serial == null)
        serials;
    };

  # subnetIp "10.20.0.0/24" 5  =>  "10.20.0.5"
  # subnetIp "192.0.2.252/30" 1 => "192.0.2.1"  (host-octet only,
  # caller knows the prefix length)
  subnetIp = subnet: octet:
    let
      base = builtins.head (lib.splitString "/" subnet);
      parts = lib.splitString "." base;
      first3 = lib.take 3 parts;
    in
    lib.concatStringsSep "." (first3 ++ [ (toString octet) ]);

  subnetPrefix = subnet: builtins.head (lib.splitString "/" subnet);
  subnetMask = subnet: lib.last (lib.splitString "/" subnet);

  # Parse "10.0.0.0/24" → { netInt = 167772160; prefix = 24; }
  # Used by cidrOverlaps below. Pure Nix — no shell, no `ip`
  # spawning at eval time. Assumes a well-formed IPv4 CIDR; the
  # callers in network.nix already gate per-env shape (/24 lan, /30
  # uplink,.0 network address). cfg.hostLanCidrs is consumer-set;
  # the helper still parses it correctly for any IPv4 CIDR.
  parseCidr = cidr:
    let
      parts = lib.splitString "/" cidr;
      octets = lib.splitString "." (builtins.head parts);
      prefix =
        if lib.length parts == 2
        then lib.toInt (lib.last parts)
        else 32;
      netInt =
        lib.foldl' (acc: o: acc * 256 + lib.toInt o) 0 octets;
    in
    { inherit netInt prefix; };

  # cidrOverlaps "10.0.0.0/24" "10.0.0.128/26" = true
  # cidrOverlaps "10.0.0.0/24" "10.0.1.0/24"   = false
  # cidrOverlaps "10.0.0.0/24" "10.0.0.0/16"   = true  (containment)
  # cidrOverlaps "10.0.0.0/24" "192.168.1.0/24" = false
  #
  # Two CIDRs overlap iff their broader prefix matches on both
  # network addresses. We compare top-N bits where N = min(prefixA,
  # prefixB) by shifting both netInts right by (32 - N) via integer
  # division. No explicit mask construction needed.
  cidrOverlaps = a: b:
    let
      A =
        let
          parts = lib.splitString "/" a;
          octets = lib.splitString "." (builtins.head parts);
          prefix =
            if lib.length parts == 2
            then lib.toInt (lib.last parts)
            else 32;
          netInt =
            lib.foldl' (acc: o: acc * 256 + lib.toInt o) 0 octets;
        in
        { inherit netInt prefix; };
      B =
        let
          parts = lib.splitString "/" b;
          octets = lib.splitString "." (builtins.head parts);
          prefix =
            if lib.length parts == 2
            then lib.toInt (lib.last parts)
            else 32;
          netInt =
            lib.foldl' (acc: o: acc * 256 + lib.toInt o) 0 octets;
        in
        { inherit netInt prefix; };
      minPrefix = if A.prefix < B.prefix then A.prefix else B.prefix;
      shift = 32 - minPrefix;
      pow2 = n:
        lib.foldl' (acc: _: acc * 2) 1 (lib.genList (i: i) n);
      divisor = pow2 shift;
      aTop = A.netInt / divisor;
      bTop = B.netInt / divisor;
    in
    aTop == bTop;

  # Deterministic MAC: 02:<hash(env+ifaceSuffix)[0..8]>:<index hex>.
  # `02` = locally-administered, unicast. Last byte = index. The
  # ifaceSuffix lets a single VM with two NICs (router VMs) get two
  # distinct MACs without index collisions: pass "up" for the
  # uplink-side NIC and "lan" for the LAN-side NIC.
  mkMac = env: ifaceSuffix: index:
    let
      h = builtins.substring 0 8 (builtins.hashString "sha256" "${env}-${ifaceSuffix}");
      pair = n: builtins.substring n 2 h;
    in
    lib.toUpper "02:${pair 0}:${pair 2}:${pair 4}:${pair 6}:${hex2 index}";

  # vmRunner — single access point for per-VM runner config that
  # processes-json.nix / closures-json.nix /
  # minijail-profiles.nix / store.nix consume. Reads from
  # `config.d2b._computed.vms.<name>.config.microvm.*` — the
  # d2b-owned per-VM evaluator output (see
  # `nixos-modules/vm-evaluator.nix`). The
  # `d2b._computed.vms.<name>` storage location is a SIBLING
  # to `d2b.vms.<name>` to avoid module-system infinite
  # recursion (host.nix's composeVm pass cannot map over cfg.vms
  # and write back to d2b.vms.<name>.computed without
  # cycling). NO upstream microvm.nix dependency.
  vmRunner = config: name:
    config.d2b._computed.${name}.config.microvm or { };

  # Sibling helper for the per-VM toplevel build.
  vmToplevel = config: name:
    config.d2b._computed.${name}.config.system.build.toplevel;

  # Sibling helper for the per-VM declared runner derivation.
  # In v1.1+ this is always null (the broker generates runner
  # argv in Rust via `packages/d2b-host/src/*_argv.rs`); the
  # helper returns null for backward compat with consumers that
  # touch the path.
  vmDeclaredRunner = _config: _name: null;

  # guestConfigForbiddenNamespaces — namespace-containment policy check
  # for the per-VM guest-editable `guestConfigFile`.
  #
  # Returns the host-owned option path(s) (under `microvm.*` /
  # `d2b.*`) that the guest file — OR ANY MODULE IT IMPORTS /
  # GENERATES — defined. An empty list means the guest file touched only
  # guest-OS options.
  #
  # Mechanism: evaluate the guest file (and its full import closure) with
  # `lib.evalModules` over the REAL nixpkgs NixOS module set, so a guest
  # module that READS a standard option (e.g.
  # `config.networking.hostName` in a `mkIf` guard) resolves instead of
  # crashing the host eval. `microvm` and `d2b` are redeclared as
  # detector options that nothing else defines, and a namespace is
  # reported iff `options.<ns>.isDefined` — i.e. the guest contributed a
  # real definition. Detection is by definition-EXISTENCE, so a guest's
  # `imports`, a `builtins.toFile`-generated module, and `_file`
  # spoofing are all caught (none can hide a definition from the option
  # system).
  #
  # SCOPE / NON-GOAL: this is a best-effort namespace-containment policy
  # lint, NOT a sound eval-time security sandbox. Two known limits, both
  # inherent to evaluating guest-authored Nix as a module and both with
  # the SAME backstop (the operator-review-and-approve trust gate — the
  # host only ever evaluates a guest file the operator reviewed via
  # `config diff` and approved) and the SAME deferred sound fix (a
  # restricted/pure evaluator whose normalized output is the only thing
  # the host consumes — see docs/adr/0024 "Future work"):
  #   1. `lib.evalModules` cannot stop an approved guest file from
  #      reading host paths at eval time (e.g. `builtins.readFile`).
  #   2. This lint evaluates the guest file over the base NixOS module
  #      set, NOT the full per-VM module stack (`vm.config`, components,
  #      framework). So the `config.*` context can differ from the real
  #      eval, which has two consequences:
  #      (a) a forbidden `microvm.*`/`d2b.*` definition gated on
  #          `lib.mkIf <cond>` where `<cond>` depends on a value the real
  #          eval sets but this context does not can evaluate false here
  #          and true in the real eval, escaping the lint (false NEGATIVE);
  #      (b) a contained guest file that READS a framework-declared option
  #          (e.g. `config.d2b.sshUser`, declared by the framework but
  #          here only an undefined `anything` detector root) fails the
  #          sandbox eval and is reported fail-closed (false POSITIVE).
  #      Sound attribution of a guest's contribution in the FULL context
  #      is not reliably expressible — definition counts conflate
  #      defaults, value comparison forces (and is perturbed by) the whole
  #      runner closure, and source-file attribution is `_file`-spoofable
  #      — hence the deferred restricted-evaluator.
  # The lint reliably catches the common case: UNCONDITIONAL host-owned
  # sets, and conditional ones whose guard resolves the same here as in
  # the real eval — from any source (`imports`, `builtins.toFile`,
  # `_file` spoofing), since detection is by definition-existence. Guest
  # files that read framework-declared `d2b.*`/`microvm.*` options
  # are not supported (they read host-owned state the guest layer should
  # not depend on).
  #
  # `pkgs` + `specialArgs` mirror what the real per-VM evaluator passes
  # so a guest config valid in the real eval applies here too. Any eval
  # failure is treated fail-closed (reported as a violation).
  guestConfigForbiddenNamespaces = { pkgs, specialArgs ? { } }: guestFile:
    let
      modulesPath = toString (pkgs.path + "/nixos/modules");
      baseModules = import (modulesPath + "/module-list.nix");
      ev = lib.evalModules {
        specialArgs = {
          inherit lib pkgs modulesPath baseModules;
          utils = import (pkgs.path + "/nixos/lib/utils.nix") {
            inherit lib pkgs;
            config = ev.config;
          };
        } // specialArgs;
        modules = baseModules ++ [
          {
            nixpkgs.pkgs = pkgs;
            nixpkgs.hostPlatform = pkgs.stdenv.hostPlatform.system;
          }
          {
            options.microvm = lib.mkOption { type = lib.types.anything; };
            options.d2b = lib.mkOption { type = lib.types.anything; };
          }
          guestFile
        ];
      };
      namesIn = ns:
        lib.optionals ev.options.${ns}.isDefined
          (lib.concatMap
            (def: map (k: "${ns}.${k}") (lib.attrNames def))
            ev.options.${ns}.definitions);
      probe = builtins.tryEval (namesIn "microvm" ++ namesIn "d2b");
    in
    if probe.success then probe.value
    else [ "<guestConfigFile failed to evaluate in the containment check>" ];
}
