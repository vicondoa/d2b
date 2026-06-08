#!/usr/bin/env bash
# tests/umask-roundtrip-eval.sh— umask end-to-end eval round-trip.
#
# Asserts that the `umask` field declared in `nixos-modules/minijail-profiles.nix`
# for each sidecar role (swtpm, gpu, video, audio) propagates end-to-end through:
#
#   minijail-profiles.nix  →  _bundle.minijailProfiles.<id>.roleProfile.umask
#   →  processesJson.data.vms[*].nodes[*].profile.umask
#
# This is the fu36 silent-pipeline-drop risk gate that D5 broker-pre-NS
# extension highlighted: if any layer in the pipeline drops the umask field,
# the sidecar socket is created with the broker's inherited umask (typically
# 0o022) instead of 0o007, causing cloud-hypervisor to fail to connect.
#
# Roles with declared non-default umask (verified against minijail-profiles.nix):
#   swtpm:  umask = 7  (0o007)
#   gpu:    umask = 7  (0o007)
#   video:  umask = 7  (0o007)
#   audio:  umask = 7  (0o007)
#
# Evaluated against a synthesized x86_64-linux nixosSystem with tpm.enable,
# graphics.enable, graphics.videoSidecar, and audio.enable all true — the
# minimal configuration that instantiates all four roles simultaneously.
#
# Exit 0 on PASS, exit 1 on FAIL.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; FAILED=$((FAILED + 1)); }

FAILED=0

log "==> tests/umask-roundtrip-eval.sh"

# ---------------------------------------------------------------------------
# Nix eval expression: synthesize a nixosSystem with swtpm + gpu + audio
# all enabled, then extract the umask value from each role's RoleProfile
# as it appears in processesJson.data (the pre-serialization Nix attrset
# that becomes /etc/nixling/processes.json at activation time).
# ---------------------------------------------------------------------------
EXPR=$(cat <<EOF
let
  flake = builtins.getFlake (toString ${ROOT});
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  nixos = nixosSystem {
    system = "x86_64-linux";
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
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
          lanSubnet    = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
        nixling.vms.umask-probe = {
          enable          = true;
          env             = "work";
          index           = 10;
          ssh.user        = "alice";
          tpm.enable      = true;
          graphics.enable = true;
          graphics.videoSidecar = true;
          audio.enable    = true;
          config = {
            networking.hostName = lib.mkDefault "umask-probe";
            users.users.alice = { isNormalUser = true; uid = 1000; };
            system.stateVersion = "25.11";
          };
        };
      })
    ];
  };
  vms   = nixos.config.nixling._bundle.processesJson.data.vms;
  vm    = builtins.head (builtins.filter (v: v.vm == "umask-probe") vms);
  nodes = vm.nodes;
  findNode = role:
    let matches = builtins.filter (n: n.role == role) nodes;
    in if matches == []
       then null
       else builtins.head matches;
  umaskOf = role:
    let node = findNode role;
    in if node == null
       then null
       else node.profile.umask or null;
in {
  swtpmUmask = umaskOf "swtpm";
  gpuUmask   = umaskOf "gpu";
  videoUmask = umaskOf "video";
  audioUmask = umaskOf "audio";
}
EOF
)

log "Evaluating processesJson.data for swtpm/gpu/audio umask fields..."
OUT=$(nix-instantiate --eval --strict --json --expr "$EXPR" 2>/dev/null) || {
  log "eval failed; cannot inspect processes.json umask fields"
  exit 1
}

SWTPM_UMASK=$(printf '%s' "$OUT" | jq -r '.swtpmUmask // "null"')
GPU_UMASK=$(printf '%s'   "$OUT" | jq -r '.gpuUmask   // "null"')
VIDEO_UMASK=$(printf '%s' "$OUT" | jq -r '.videoUmask // "null"')
AUDIO_UMASK=$(printf '%s' "$OUT" | jq -r '.audioUmask // "null"')

log "swtpm.profile.umask  = ${SWTPM_UMASK}"
log "gpu.profile.umask    = ${GPU_UMASK}"
log "video.profile.umask  = ${VIDEO_UMASK}"
log "audio.profile.umask  = ${AUDIO_UMASK}"

# Expected value: 7 (decimal encoding of 0o007).
EXPECTED=7

if [ "$SWTPM_UMASK" = "$EXPECTED" ]; then
  ok "swtpm umask = ${SWTPM_UMASK} (0o007) — matches minijail-profiles.nix declaration"
else
  fail "swtpm umask = ${SWTPM_UMASK} (expected ${EXPECTED}); fu36 pipeline drop detected"
fi

if [ "$GPU_UMASK" = "$EXPECTED" ]; then
  ok "gpu umask = ${GPU_UMASK} (0o007) — matches minijail-profiles.nix declaration"
else
  fail "gpu umask = ${GPU_UMASK} (expected ${EXPECTED}); fu36 pipeline drop detected"
fi

if [ "$VIDEO_UMASK" = "$EXPECTED" ]; then
  ok "video umask = ${VIDEO_UMASK} (0o007) — matches minijail-profiles.nix declaration"
else
  fail "video umask = ${VIDEO_UMASK} (expected ${EXPECTED}); fu36 pipeline drop detected"
fi

if [ "$AUDIO_UMASK" = "$EXPECTED" ]; then
  ok "audio umask = ${AUDIO_UMASK} (0o007) — matches minijail-profiles.nix declaration"
else
  fail "audio umask = ${AUDIO_UMASK} (expected ${EXPECTED}); fu36 pipeline drop detected"
fi

if [ "$FAILED" -gt 0 ]; then
  log "==> FAIL: ${FAILED} role(s) have a umask pipeline drop (D17/P2.6 fu36)"
  exit 1
fi

log "==> PASS: swtpm, gpu, video, audio umask = 7 (0o007) end-to-end (D17/P2.6 fu36)"
