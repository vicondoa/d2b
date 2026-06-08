#!/usr/bin/env bash
# tests/readiness-waves-eval.sh — eval-time gate that verifies the
# p0..p7 daemon-only rollout waves are present in the
# defaultSwitchReadiness option schema.
#
# Why: p0..p7 entries in readinessWaveSpecs ensure
# that nixling.daemonExperimental.enable auto-flips to true only once
# every phase is both implemented AND validated.  Regressions that
# accidentally drop a wave (e.g. a merge conflict resolving the attrset
# to an older snapshot) would silently break the allReady gate and
# either prevent the daemon from enabling itself or enable it
# prematurely.
#
# Shape: eval-only, no live host required (mirrors usbip-gating-eval.sh
# and net-vm-network-eval.sh). Wired into tests/static.sh.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/readiness-waves-eval.sh"

# Build an eval expression that returns every wave name present in the
# defaultSwitchReadiness option attrset.  We use a minimal host config
# so the eval is as fast as possible; no VMs required.
EXPR=$(cat <<EOF
let
  flake = builtins.getFlake (toString $ROOT);
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
          lanSubnet = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
      })
    ];
  };
  waveNames = builtins.attrNames nixos.config.nixling.defaultSwitchReadiness;
  hasWave = w: builtins.elem w waveNames;
in {
  waveNames    = waveNames;
  hasP0        = hasWave "p0";
  hasP1        = hasWave "p1";
  hasP2        = hasWave "p2";
  hasP3        = hasWave "p3";
  hasP4        = hasWave "p4";
  hasP5        = hasWave "p5";
  hasP6        = hasWave "p6";
  hasP7        = hasWave "p7";
  p0ImplDef    = nixos.config.nixling.defaultSwitchReadiness.p0.implemented;
  p0ValidDef   = nixos.config.nixling.defaultSwitchReadiness.p0.validated;
  daemonAutoDefault = nixos.config.nixling.daemonExperimental.enable;
}
EOF
)

OUT=$(nix-instantiate --eval --strict --json --expr "$EXPR" 2>/dev/null) || \
  fail "eval failed; cannot inspect readinessWaveSpecs"

check_bool() {
  local key="$1" expected="$2"
  local val
  val=$(printf '%s' "$OUT" | jq -r --arg k "$key" '.[$k]')
  if [ "$val" = "$expected" ]; then
    ok "$key = $expected"
  else
    fail "$key: expected $expected, got $val"
  fi
}

check_bool "hasP0"  "true"
check_bool "hasP1"  "true"
check_bool "hasP2"  "true"
check_bool "hasP3"  "true"
check_bool "hasP4"  "true"
check_bool "hasP5"  "true"
check_bool "hasP6"  "true"
check_bool "hasP7"  "true"

# p0..p7 default to implemented=false so the daemon doesn't auto-enable
# until each phase is explicitly shipped and validated.
check_bool "p0ImplDef"  "false"
check_bool "p0ValidDef" "false"

# With all p0..p7 unimplemented, daemonExperimental.enable must default
# to false (allReady gate is false).
check_bool "daemonAutoDefault" "false"

log "==> readiness-waves-eval OK"
