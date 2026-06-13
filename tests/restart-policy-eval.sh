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
#   - systemd.services."nixling-<vm>-video"                  (per-VM)
#   - systemd.services."nixling-<vm>-gpu"                    (per-VM)
#   - systemd.services."nixling-otel-relay@"                 (template)
#
# Wave-1 observability extends the allowlist with:
#   - systemd.services."nixling-otel-relay@"                 (template)
#   - systemd.services."nixling-otel-host-bridge"            (singleton)
#   - systemd.services."nixling-ch-exporter"                 (singleton)
#   - workload guest systemd.services."nixling-otel-vsock-out"
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
  flake = builtins.getFlake "git+file://$ROOT";
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
  hostSvcs = nixos.config.systemd.services;
  guestSvcs =
    if nixos.config ? microvm && nixos.config.microvm ? vms && nixos.config.microvm.vms ? "full-vm"
    then nixos.config.microvm.vms."full-vm".config.config.systemd.services
    else {};
  obsGuestSvcs =
    if nixos.config ? microvm && nixos.config.microvm ? vms && nixos.config.microvm.vms ? "sys-obs"
    then nixos.config.microvm.vms."sys-obs".config.config.systemd.services
    else {};
  pullFrom = services: key:
    if !(builtins.hasAttr key services) then null
    else
      let s = services.\${key};
      in {
        ric = s.restartIfChanged or null;
        xric = (s.unitConfig or {}).X-RestartIfChanged or null;
      };
  pull = key: pullFrom hostSvcs key;
  pullGuest = key: pullFrom guestSvcs key;
  pullObsGuest = key: pullFrom obsGuestSvcs key;
in {
  "nixling@" = pull "nixling@";
  "microvm@" = pull "microvm@";
  "microvm-virtiofsd@full-vm" = pull "microvm-virtiofsd@full-vm";
  "nixling-full-vm-swtpm" = pull "nixling-full-vm-swtpm";
  "nixling-full-vm-snd"   = pull "nixling-full-vm-snd";
  "nixling-full-vm-video" = pull "nixling-full-vm-video";
  "nixling-full-vm-gpu"   = pull "nixling-full-vm-gpu";
  "nixling-otel-relay@" = pull "nixling-otel-relay@";
  "nixling-otel-host-bridge" = pull "nixling-otel-host-bridge";
  "nixling-ch-exporter" = pull "nixling-ch-exporter";
  "guest:nixling-otel-vsock-out" = pullGuest "nixling-otel-vsock-out";
  "obs:nixling-otel-vsock-in-host" = pullObsGuest "nixling-otel-vsock-in-host";
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

check_optional "nixling@" "daemon supervisor + broker SpawnRunner{CloudHypervisor}"
check_optional "microvm@" "upstream microvm template may be absent on daemon-only hosts"
check_optional "microvm-virtiofsd@full-vm" "virtiofsd runner is broker-supervised on daemon-only hosts"
check_optional "nixling-full-vm-swtpm" "broker SpawnRunner{Swtpm}"
# The per-VM nixling-<vm>-snd /
# -video / -gpu sidecars and the nixling-otel-relay@ template have
# been deleted; their replacements are broker `SpawnRunner` runners
# (Audio / Video / Gpu / OtelHostBridge) and carry no
# `restartIfChanged` knob (broker `supervisor::pidfd` owns the
# restart contract). The `nixling-ch-exporter` host singleton was
# replaced by the daemon's `/metrics` endpoint and is gone for the
# same reason. Checks retained: the upstream `microvm@` and
# `microvm-virtiofsd@` per-VM templates (still emitted by upstream
# microvm.nix via the `microvm.vms` translation in host.nix), the
# per-VM swtpm sidecar (host-sidecars.nix deletion in took the
# top-level `nixling-<vm>-swtpm` service with it — listed here only
# so a re-introduction regresses the gate), and the in-guest
# observability vsock relays.
check_optional "nixling-full-vm-snd"   "broker SpawnRunner{Audio}"
check_optional "nixling-full-vm-video" "broker SpawnRunner{Video}"
check_optional "nixling-full-vm-gpu"   "broker SpawnRunner{Gpu}"
check_optional "nixling-otel-relay@"   "broker SpawnRunner{OtelHostBridge}"
check_optional "nixling-otel-host-bridge" "broker SpawnRunner{OtelHostBridge}"
check_optional "nixling-ch-exporter"   "folded into nixlingd /metrics"
check_optional "guest:nixling-otel-vsock-out" "guest observability relay may be absent on daemon-only hosts"
check_optional "obs:nixling-otel-vsock-in-host" "observability VM relay may be absent on daemon-only hosts"

# In-VM observability units (nixling-otel-vsock-out.service in workload
# guests and nixling-otel-vsock-in-*.service in the obs VM) are out of
# scope here: this file evaluates host systemd.services only. Future
# in-VM tests will cover their restartIfChanged policy.

# The obs VM's nixling-otel-vsock-in-*.service units are still covered in
# observability-eval.sh, which evaluates the stack VM's guest config.

# Assert that nixlingd.service
# carries restartIfChanged = false when daemonExperimental.enable = true.
# nixlingd is the long-lived supervisor whose pidfd owns the child runner
# DAG; a rebuild-triggered restart would tear down all in-flight VM
# processes. The VM lifecycle policy (AGENTS.md) extends to the daemon.
EXPR_DAEMON=$(cat <<'EOFD'
let
  flake = builtins.getFlake "git+file://ROOT_PLACEHOLDER";
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
        # Force daemon on explicitly so the unit materialises regardless
        # of whether any allReady gates have flipped.
        nixling.daemonExperimental.enable = true;
      })
    ];
  };
  svc = nixos.config.systemd.services.nixlingd or null;
in {
  present        = svc != null;
  ric            = if svc != null then svc.restartIfChanged or null else null;
  xric           = if svc != null then (svc.unitConfig or {}).X-RestartIfChanged or null else null;
}
EOFD
)
# Substitute $ROOT into the heredoc (single-quote prevented expansion above).
EXPR_DAEMON="${EXPR_DAEMON//ROOT_PLACEHOLDER/$ROOT}"

OUT_DAEMON=$(nix-instantiate --eval --strict --json --expr "$EXPR_DAEMON" 2>/dev/null) || \
  fail "daemon eval failed; cannot inspect nixlingd.service restart policy"

present=$(printf '%s' "$OUT_DAEMON" | jq -r '.present')
if [ "$present" != "true" ]; then
  fail "nixlingd.service: service not found in config when daemonExperimental.enable = true"
fi

ric_daemon=$(printf '%s' "$OUT_DAEMON" | jq -r '.ric')
xric_daemon=$(printf '%s' "$OUT_DAEMON" | jq -r '.xric')

case "$ric_daemon" in
  false) ok "nixlingd.service: restartIfChanged = false" ;;
  *)
    if [ "$xric_daemon" != "null" ]; then
      fail "nixlingd.service: uses unitConfig.X-RestartIfChanged ($xric_daemon) which NixOS switch-to-configuration ignores (emits under [Unit] not [Service]). Use top-level \`restartIfChanged = false\` instead. See AGENTS.md 'Adding new per-VM units'."
    fi
    fail "nixlingd.service: missing restartIfChanged=false (got ric=$ric_daemon). The VM lifecycle policy extends to the daemon — see AGENTS.md."
    ;;
esac

log "==> restart-policy-eval OK"
