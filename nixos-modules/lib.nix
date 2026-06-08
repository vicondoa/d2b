# Shared pure helpers for the nixling framework. Imported as a
# function (`import ./lib.nix { inherit lib; }`) by network.nix and
# host.nix so they share the same MAC/IP derivation rules.
#
# Pass `pkgs` as well (`import ./lib.nix { inherit lib pkgs; }`) to
# get `nixlingReadAudioState`, a Nix-store shell fragment that both
# audio.nix and cli.nix source for fail-closed audio-state reads.
{ lib, pkgs ? null }:

let
  hex2 = i:
    let s = lib.toHexString i;
    in if lib.stringLength s == 1 then "0${s}" else s;

  # nixling_read_audio_state <vm>
  # ------------------------------------------------------------
  # Fail-closed reader for /var/lib/nixling/<vm>/audio-state.json.
  # Output (one line on stdout): "mic=<on|off> speaker=<on|off>".
  # NEVER exits non-zero — callers (extraArgsScript, nixling CLI)
  # cannot handle a non-zero exit mid-flow.
  #
  # Returns "mic=off speaker=off" for EVERY error case:
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
  # nixling shell application (jq also in runtimeInputs, harmless).
  nixlingReadAudioState =
    if pkgs == null then null
    else
      pkgs.writeText "nixling-read-audio-state.sh" ''
        nixling_read_audio_state() {
          local _nas_vm="$1" _nas_f _nas_mic=off _nas_spk=off _nas_raw
          local _nas_canonical _nas_expected _nas_stat
          # security-2: state file lives under the root-owned state/ subdir.
          # Phase 2a Pass 2: VM state dir moved under vms/<vm>/.
          _nas_f="/var/lib/nixling/vms/$_nas_vm/state/audio-state.json"
          _nas_expected="/var/lib/nixling/vms/$_nas_vm/state/audio-state.json"
          # Canonicalize: fail closed if path doesn't resolve or is a symlink
          # pointing outside the expected location.
          _nas_canonical=$(realpath -e "$_nas_f" 2>/dev/null) \
            || { printf 'mic=off speaker=off\n'; return 0; }
          [ "$_nas_canonical" = "$_nas_expected" ] \
            || { printf 'mic=off speaker=off\n'; return 0; }
          # Verify ownership and mode: must be root:nixling-launcher 640.
          _nas_stat=$(stat -c '%U %G %a' "$_nas_canonical" 2>/dev/null) \
            || { printf 'mic=off speaker=off\n'; return 0; }
          [ "$_nas_stat" = "root nixling-launcher 640" ] \
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
            # (e.g. an interactive operator or nixling-gpu-<vm> before ACLs are
            # applied). Fail closed so the sidecar never gets audio access on
            # a permission error.
            printf 'nixling: audio-state unreadable for %s (permission denied) — failing closed\n' "$_nas_vm" >&2
          fi
          printf 'mic=%s speaker=%s\n' "$_nas_mic" "$_nas_spk"
        }
      '';
in
{
  inherit hex2;
  inherit nixlingReadAudioState;

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
  # uplink, .0 network address). cfg.hostLanCidrs is consumer-set;
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

  # v1.1-final: vmRunner — single access point for per-VM runner
  # config that processes-json.nix / closures-json.nix /
  # minijail-profiles.nix / store.nix consume. Reads from
  # `config.nixling._computed.vms.<name>.config.microvm.*` — the
  # nixling-owned per-VM evaluator output (see
  # `nixos-modules/vm-evaluator.nix`). The
  # `nixling._computed.vms.<name>` storage location is a SIBLING
  # to `nixling.vms.<name>` to avoid module-system infinite
  # recursion (host.nix's composeVm pass cannot map over cfg.vms
  # and write back to nixling.vms.<name>.computed without
  # cycling). NO upstream microvm.nix dependency.
  vmRunner = config: name:
    config.nixling._computed.${name}.config.microvm or { };

  # Sibling helper for the per-VM toplevel build.
  vmToplevel = config: name:
    config.nixling._computed.${name}.config.system.build.toplevel;

  # Sibling helper for the per-VM declared runner derivation.
  # In v1.1+ this is always null (the broker generates runner
  # argv in Rust via `packages/nixling-host/src/*_argv.rs`); the
  # helper returns null for backward compat with consumers that
  # touch the path.
  vmDeclaredRunner = _config: _name: null;
}
