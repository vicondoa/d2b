#!/usr/bin/env bash
# tests/autostart-wiring-eval.sh — positive-eval regression test for
# Spec corrections #32 (v0.1.3) and #33 (v0.1.3/v0.1.6 SWArch-M10).
#
# Both correctness invariants must hold:
#
#   H3 / #32 — `nixling@<vm>` is a TEMPLATE-only service. Declaring
#     `systemd.services."nixling@${name}"` per-VM (instead of the
#     `systemd.targets.multi-user.wants = [ "nixling@<vm>.service" ]`
#     symlink path) materializes a separate unit file lacking the
#     template's ExecStart/ExecStop, which systemd then refuses with
#     "Service has no ExecStart=, ExecStop=, or SuccessAction=". So
#     for an autostart=true VM:
#       - `systemd.services."nixling@<vm>"` MUST NOT exist as an
#         attr (only `"nixling@"` does);
#       - `systemd.targets.multi-user.wants` MUST list
#         `"nixling@<vm>.service"`.
#
#   H4 / #33 + SWArch-M10 (v0.1.6) — `systemd.targets.microvms.wants`
#     MUST be `[]`. v0.1.3 originally narrowed it to the autostart=true
#     subset; v0.1.6 narrows further to `[]`, so all autostart wiring
#     funnels exclusively through `multi-user.target.wants ->
#     nixling@<vm>.service` (single boot path; no duplicate
#     microvm@<vm> direct-pull).
#
# Wired into tests/static.sh.

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/autostart-wiring-eval.sh"

# Synthesize one autostart=true and one autostart=false workload VM,
# plus the framework's per-env net VM (synthesized by network.nix
# from `nixling.envs.work`). After v0.1.6 SWArch-M10 the net VM is
# the ONLY entity that needs an early boot path and its
# `nixling-work-net.service` is wired via the env-router code path,
# not microvms.target.wants — so we expect microvms.target.wants to
# be the empty list.
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
        nixling.site = { waylandUser = "alice"; launcherUsers = [ "alice" ]; yubikey.enable = false; };
        nixling.envs.work = { lanSubnet = "10.20.0.0/24"; uplinkSubnet = "192.0.2.0/30"; };
        nixling.vms.auto-vm = {
          enable = true;
          env = "work";
          index = 10;
          autostart = true;
          ssh.user = "alice";
          config = { lib, ... }: {
            networking.hostName = lib.mkDefault "auto-vm";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
        nixling.vms.manual-vm = {
          enable = true;
          env = "work";
          index = 11;
          autostart = false;
          ssh.user = "alice";
          config = { lib, ... }: {
            networking.hostName = lib.mkDefault "manual-vm";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
      })
    ];
  };
  svcs = nixos.config.systemd.services;
  mu = nixos.config.systemd.targets.multi-user.wants;
  mvms = nixos.config.systemd.targets.microvms.wants;
in {
  hasNixlingTemplate = builtins.hasAttr "nixling@" svcs;
  hasPerVmNixlingAuto = builtins.hasAttr "nixling@auto-vm" svcs;
  hasPerVmNixlingManual = builtins.hasAttr "nixling@manual-vm" svcs;
  muHasAuto = builtins.elem "nixling@auto-vm.service" mu;
  muHasManual = builtins.elem "nixling@manual-vm.service" mu;
  microvmsWants = mvms;
}
EOF
)

OUT=$(nix-instantiate --eval --strict --json --expr "$EXPR" 2>/dev/null) || \
  fail "eval failed; cannot inspect autostart wiring"

HAS_TPL=$(printf '%s' "$OUT"     | jq -r '.hasNixlingTemplate')
HAS_AUTO=$(printf '%s' "$OUT"    | jq -r '.hasPerVmNixlingAuto')
HAS_MANUAL=$(printf '%s' "$OUT"  | jq -r '.hasPerVmNixlingManual')
MU_AUTO=$(printf '%s' "$OUT"     | jq -r '.muHasAuto')
MU_MANUAL=$(printf '%s' "$OUT"   | jq -r '.muHasManual')
MVMS=$(printf '%s' "$OUT"        | jq -c '.microvmsWants')

# H3a: template must exist.
if [ "$HAS_TPL" != "true" ]; then
  fail "systemd.services.\"nixling@\" template missing (host-wrapper.nix:57)"
fi
ok "systemd.services.\"nixling@\" template exists"

# H3b: per-instance attrs MUST NOT exist (template-only invariant).
if [ "$HAS_AUTO" != "false" ]; then
  fail "systemd.services.\"nixling@auto-vm\" attr exists; it MUST be template-only (autostart wired via multi-user.target.wants). See Spec correction #32."
fi
ok "systemd.services.\"nixling@auto-vm\" attr correctly ABSENT (template-only)"
if [ "$HAS_MANUAL" != "false" ]; then
  fail "systemd.services.\"nixling@manual-vm\" attr exists; it MUST be template-only."
fi
ok "systemd.services.\"nixling@manual-vm\" attr correctly ABSENT (template-only)"

# H3c: multi-user.target.wants pulls autostart=true ONLY.
if [ "$MU_AUTO" != "true" ]; then
  fail "multi-user.target.wants is missing 'nixling@auto-vm.service' (autostart=true VM not wired). See host-wrapper.nix:96-98."
fi
ok "multi-user.target.wants includes 'nixling@auto-vm.service'"
if [ "$MU_MANUAL" != "false" ]; then
  fail "multi-user.target.wants contains 'nixling@manual-vm.service' but autostart=false; should NOT be present."
fi
ok "multi-user.target.wants excludes 'nixling@manual-vm.service' (autostart=false)"

# H4 / SWArch-M10: microvms.target.wants must be [].
if [ "$MVMS" != "[]" ]; then
  fail "systemd.targets.microvms.wants = $MVMS; expected [] after v0.1.6 SWArch-M10. Upstream microvm.nix's wants cascade is suppressed via lib.mkForce; autostart wiring goes through multi-user.target → nixling@<vm>.service exclusively."
fi
ok "systemd.targets.microvms.wants is [] (post-SWArch-M10: single autostart path via multi-user.target)"

log "==> autostart-wiring-eval OK"
