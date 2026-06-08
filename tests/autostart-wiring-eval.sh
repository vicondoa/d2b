#!/usr/bin/env bash
# tests/autostart-wiring-eval.sh — positive-eval regression test for
# autostart wiring after ph6-remove-systemd-emission.
#
# Pre-P6 (ph6-remove-systemd-emission) this gate locked in Spec
# correction #32 + SWArch-M10: the `nixling@<vm>.service` template
# was the autostart driver, wired via
# `systemd.targets.multi-user.wants`, and `microvms.target.wants`
# had to be `[]` to suppress the upstream `microvm@<vm>` autostart
# cascade.
#
# After P6 the `nixling@<vm>.service` template is DELETED
# (host-wrapper.nix removed). Autostart is driven by the
# `nixlingd.service` daemon, which reads `nixling.vms.<name>.autostart`
# out of `/etc/nixling/bundle.json` and brings VMs up via the
# `SpawnRunner{role: CloudHypervisor}` broker op. This gate is
# therefore the *inverse* of its pre-P6 form:
#
#   - `systemd.services."nixling@"` MUST NOT exist (template deleted);
#   - `systemd.targets.multi-user.wants` MUST NOT pull any
#     `nixling@*.service` (template gone, dangling wants would
#     produce a "Failed to load configuration" warning at boot);
#   - `systemd.targets.microvms.wants` MUST still be `[]` — even
#     though the `nixling@` driver is gone, the `microvm.vms`
#     translation in host.nix is preserved (deferred follow-up,
#     see CHANGELOG.md). The upstream `microvm@<vm>.service` units
#     it emits would still autostart via microvms.target unless we
#     explicitly suppress that cascade.
#   - `nixlingd.service` MUST be wired into `multi-user.target.wants`
#     so it comes up on boot and drives `autostart=true` VMs.
#
# Wired into tests/static.sh.

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/autostart-wiring-eval.sh"

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
        nixling.daemonExperimental.enable = true;
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
  nlAttrs = builtins.filter (n: builtins.match "^nixling@.*\\\\.service$" n != null) mu;
in {
  hasNixlingTemplate = builtins.hasAttr "nixling@" svcs;
  hasPerVmNixlingAuto = builtins.hasAttr "nixling@auto-vm" svcs;
  hasPerVmNixlingManual = builtins.hasAttr "nixling@manual-vm" svcs;
  muNixlingAtEntries = nlAttrs;
  hasNixlingd = builtins.hasAttr "nixlingd" svcs;
  nixlingdWantedBy = if builtins.hasAttr "nixlingd" svcs then svcs.nixlingd.wantedBy or [ ] else [ ];
  microvmsWants = mvms;
}
EOF
)

OUT=$(nix-instantiate --eval --strict --json --expr "$EXPR" 2>/dev/null) || \
  fail "eval failed; cannot inspect autostart wiring"

HAS_TPL=$(printf '%s' "$OUT"     | jq -r '.hasNixlingTemplate')
HAS_AUTO=$(printf '%s' "$OUT"    | jq -r '.hasPerVmNixlingAuto')
HAS_MANUAL=$(printf '%s' "$OUT"  | jq -r '.hasPerVmNixlingManual')
MU_NL=$(printf '%s' "$OUT"       | jq -c '.muNixlingAtEntries')
HAS_NLD=$(printf '%s' "$OUT"     | jq -r '.hasNixlingd')
NLD_WB=$(printf '%s' "$OUT"      | jq -c '.nixlingdWantedBy')
MVMS=$(printf '%s' "$OUT"        | jq -c '.microvmsWants')

# P6 H3a-inverted: nixling@ template MUST NOT exist (host-wrapper.nix
# removed by ph6-remove-systemd-emission).
if [ "$HAS_TPL" != "false" ]; then
  fail "systemd.services.\"nixling@\" template still present; should be deleted by ph6-remove-systemd-emission (host-wrapper.nix removed). Replacement: nixlingd.service drives per-VM SpawnRunner{role: CloudHypervisor} via broker."
fi
ok "systemd.services.\"nixling@\" template correctly ABSENT (P6 deletion)"

# P6 H3b: per-instance attrs MUST NOT exist either.
if [ "$HAS_AUTO" != "false" ]; then
  fail "systemd.services.\"nixling@auto-vm\" attr exists; should be absent post-P6 (no nixling@<vm> path)."
fi
ok "systemd.services.\"nixling@auto-vm\" correctly ABSENT"
if [ "$HAS_MANUAL" != "false" ]; then
  fail "systemd.services.\"nixling@manual-vm\" attr exists; should be absent post-P6."
fi
ok "systemd.services.\"nixling@manual-vm\" correctly ABSENT"

# P6 H3c: no nixling@*.service entries in multi-user.target.wants.
if [ "$MU_NL" != "[]" ]; then
  fail "multi-user.target.wants still contains nixling@*.service entries: $MU_NL. The nixling@ template is deleted; dangling wants would log a load failure at every boot."
fi
ok "multi-user.target.wants has no dangling nixling@*.service entries"

# P6 H4: nixlingd.service is the new autostart driver.
if [ "$HAS_NLD" != "true" ]; then
  fail "systemd.services.\"nixlingd\" missing; nixlingd is the post-P6 autostart driver."
fi
ok "systemd.services.\"nixlingd\" present"
if ! printf '%s' "$NLD_WB" | jq -e 'index("multi-user.target")' >/dev/null; then
  fail "nixlingd.service is not wired to multi-user.target (wantedBy missing); daemon won't autostart on boot. Got wantedBy=$NLD_WB"
fi
ok "nixlingd.service wired to multi-user.target (P6 autostart driver)"

# H4 (preserved): microvms.target.wants must be [].
if [ "$MVMS" != "[]" ]; then
  fail "systemd.targets.microvms.wants = $MVMS; expected [] (upstream microvm.nix's autostart cascade must stay suppressed even though the host.nix microvm.vms translation is preserved as a deferred follow-up). See host.nix lib.mkForce [] block."
fi
ok "systemd.targets.microvms.wants is [] (upstream microvm@ cascade suppressed)"

log "==> autostart-wiring-eval OK"
