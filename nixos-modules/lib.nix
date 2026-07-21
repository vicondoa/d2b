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

  # vmRunner — single access point for per-workload runner config.
  # Reads from `config.d2b._computedWorkloads.<workloadId>.config.microvm.*`
  # — the d2b-owned per-workload evaluator output (see
  # `nixos-modules/vm-evaluator.nix`, composed by `host.nix`). No
  # upstream microvm.nix dependency, and no read of the removed
  # `d2b.vms`/`d2b._computed` (singular) surface: real consumers
  # (processes-json.nix, closures-json.nix) already read
  # `cfg._computedWorkloads.${workloadId}` directly, and this helper
  # exists as the equivalent named access point for callers that
  # prefer it.
  vmRunner = config: workloadId:
    config.d2b._computedWorkloads.${workloadId}.config.microvm or { };

  # Sibling helper for the per-workload toplevel build.
  vmToplevel = config: workloadId:
    config.d2b._computedWorkloads.${workloadId}.config.system.build.toplevel;

  # Sibling helper for the per-workload declared runner derivation.
  # In v1.1+ this is always null (the broker generates runner
  # argv in Rust via `packages/d2b-host/src/*_argv.rs`); the
  # helper returns null for backward compat with consumers that
  # touch the path.
  vmDeclaredRunner = _config: _workloadId: null;
}
