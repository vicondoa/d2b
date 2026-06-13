#!/usr/bin/env bash
# Build-time smoke for patched CH + patched crosvm video command surface.
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log() { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok() { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/video-binary-contract.sh"

expr=$(cat <<EOF
let
  flake = builtins.getFlake "git+file://$ROOT";
  nixos = flake.inputs.nixpkgs.lib.nixosSystem {
    system = "x86_64-linux";
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        nixpkgs.config.allowUnfree = true;
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
        nixling.vms.demo-gfx = {
          enable = true;
          env = "work";
          index = 11;
          ssh.user = "alice";
          graphics.enable = true;
          graphics.videoSidecar = true;
          config = {
            networking.hostName = lib.mkDefault "demo-gfx";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
      })
    ];
  };
in nixos.config.nixling._bundle.processesJson.path
EOF
)

processes_json=$(nix build --no-link --print-out-paths --impure --expr "$expr")
video_bin=$(jq -r '.vms[] | select(.vm == "demo-gfx") | .nodes[] | select(.id == "video") | .binaryPath' "$processes_json")
ch_bin=$(jq -r '.vms[] | select(.vm == "demo-gfx") | .nodes[] | select(.id == "cloud-hypervisor") | .binaryPath' "$processes_json")

[ -x "$video_bin" ] || fail "video crosvm binary missing: $video_bin"
[ -x "$ch_bin" ] || fail "cloud-hypervisor binary missing: $ch_bin"

"$video_bin" device video-decoder --help 2>&1 | grep -q -- '--backend' \
  || fail "patched crosvm does not expose device video-decoder --backend"
"$ch_bin" --help 2>&1 | grep -q -- '--vhost-user-media' \
  || fail "patched Cloud Hypervisor does not expose --vhost-user-media"

ok "patched crosvm video-decoder and patched Cloud Hypervisor media flag are available"
