#!/usr/bin/env bash
# tests/restart-policy-eval.sh — regression test for Spec correction
# #37 (v0.1.5: every per-VM lifecycle service in the framework carries
# `restartIfChanged = false`).
#
# Pre-v0.1.5, `nixos-rebuild switch` cycled per-VM units mid-flight:
#   * graphics VMs — the GPU sidecar IS the cloud-hypervisor process,
#     so its restart terminated CH, evaporating in-RAM Entra
#     device-bound tokens and the user's login session.
#   * headless VMs — every framework-touched config (host-keys
#     refresh wiring, virtiofsd hardening) caused NixOS to override
#     upstream microvm.nix's `X-RestartIfChanged=false` back to `true`.
#
# Baseline services that must opt out (see CHANGELOG.md v0.1.5 entry):
#
#   - systemd.services."nixling@"                            (template)
#   - systemd.services."microvm@"                            (template,
#       framework override in host-known-hosts.nix)
#   - systemd.services."microvm-virtiofsd@<vm>"              (per-VM)
#   - systemd.services."nixling-<vm>-swtpm"                  (per-VM)
#   - systemd.services."nixling-<vm>-snd"                    (per-VM)
#   - systemd.services."nixling-<vm>-gpu"                    (per-VM)
#   - systemd.services."nixling-otel-relay@"                 (template)
#
# Wave-1 observability extends the host-side allowlist with:
#   - systemd.services."nixling-otel-relay@"                 (template)
#   - systemd.services."nixling-otel-host-bridge"            (singleton)
#   - systemd.services."nixling-ch-exporter"                 (singleton)
#
# v0.1.7 tightened the invariant: top-level `restartIfChanged = false`
# is REQUIRED. `unitConfig.X-RestartIfChanged = false` is rejected
# because NixOS emits it under [Unit], where switch-to-configuration
# ignores it.
# Wired into tests/static.sh.

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
skip() { log "  SKIP: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/restart-policy-eval.sh"

# Synthesize a single workload VM with graphics + audio + TPM all
# enabled so EVERY per-VM lifecycle service materialises in one eval.
EXPR=$(cat <<EOF2
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
        nixling.site = { waylandUser = "alice"; launcherUsers = [ "alice" ]; yubikey.enable = false; };
        nixling.envs.work = { lanSubnet = "10.20.0.0/24"; uplinkSubnet = "192.0.2.0/30"; };
        nixling.observability.enable = true;
        nixling.vms.full-vm = {
          enable = true;
          env = "work";
          index = 10;
          ssh.user = "alice";
          graphics.enable = true;
          audio.enable = true;
          tpm.enable = true;
          observability.enable = true;
          config = { lib, ... }: {
            networking.hostName = lib.mkDefault "full-vm";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
      })
    ];
  };
  svcs = nixos.config.systemd.services;
  pull = key:
    if !(builtins.hasAttr key svcs) then null
    else
      let s = svcs.\${key};
      in {
        ric = s.restartIfChanged or null;
        xric = (s.unitConfig or {}).X-RestartIfChanged or null;
      };
in {
  "nixling@" = pull "nixling@";
  "microvm@" = pull "microvm@";
  "microvm-virtiofsd@full-vm" = pull "microvm-virtiofsd@full-vm";
  "nixling-full-vm-swtpm" = pull "nixling-full-vm-swtpm";
  "nixling-full-vm-snd"   = pull "nixling-full-vm-snd";
  "nixling-full-vm-gpu"   = pull "nixling-full-vm-gpu";
  "nixling-otel-relay@" = pull "nixling-otel-relay@";
  "nixling-otel-host-bridge" = pull "nixling-otel-host-bridge";
  "nixling-ch-exporter" = pull "nixling-ch-exporter";
}
EOF2
)

OUT=$(nix-instantiate --eval --strict --json --expr "$EXPR" 2>/dev/null) || \
  fail "eval failed; cannot inspect restart policy"

# v0.1.7+: REQUIRE top-level `restartIfChanged == false` (NixOS option,
# emitted as `[Service] X-RestartIfChanged=false`). The broken
# `unitConfig.X-RestartIfChanged = false` shape emits under [Unit],
# which NixOS's switch-to-configuration ignores.
check() {
  local key="$1"
  local entry ric xric
  entry=$(printf '%s' "$OUT" | jq -c --arg k "$key" '.[$k]')
  if [ "$entry" = "null" ]; then
    fail "$key: service not found in config (declaration regressed?)"
  fi
  ric=$(printf '%s' "$entry" | jq -r '.ric')
  xric=$(printf '%s' "$entry" | jq -r '.xric')
  case "$ric" in
    false) ok "$key: restartIfChanged = false"; return ;;
  esac
  if [ "$xric" != "null" ]; then
    fail "$key: uses unitConfig.X-RestartIfChanged ($xric) which NixOS switch-to-configuration ignores (emits under [Unit] not [Service]). Use top-level \`restartIfChanged = false\` instead. See CHANGELOG.md v0.1.7 'unitConfig.X-RestartIfChanged silently ignored — replaced with restartIfChanged'."
  fi
  fail "$key: missing restartIfChanged=false (got ric=$ric). See CHANGELOG.md v0.1.5 + v0.1.7."
}

check_optional() {
  local key="$1" why="$2"
  local entry
  entry=$(printf '%s' "$OUT" | jq -c --arg k "$key" '.[$k]')
  if [ "$entry" = "null" ]; then
    skip "$key: TODO post-integration — $why"
    return 0
  fi
  check "$key"
}

check "nixling@"
check "microvm@"
check "microvm-virtiofsd@full-vm"
check "nixling-full-vm-swtpm"
check "nixling-full-vm-snd"
check "nixling-full-vm-gpu"
check "nixling-otel-relay@"
check "nixling-otel-host-bridge"
check "nixling-ch-exporter"

# In-VM observability units (nixling-otel-vsock-out.service in workload
# guests and nixling-otel-vsock-in.service in the obs VM) are out of
# scope here: this file evaluates host systemd.services only. Future
# in-VM tests will cover their restartIfChanged policy.

log "==> restart-policy-eval OK"
