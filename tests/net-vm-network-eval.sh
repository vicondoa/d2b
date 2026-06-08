#!/usr/bin/env bash
# tests/net-vm-network-eval.sh — regression test for W5 H1.
#
# Verifies that auto-instantiated net VMs neutralize the catch-all
# `10-eth-dhcp` network from `nixos-modules/base.nix` so it cannot
# preempt the per-MAC `10-uplink` / `10-lan` static configuration in
# `nixos-modules/net.nix`.
#
# Assertions:
#   1. `10-eth-dhcp.matchConfig.Type` MUST NOT be `"ether"` (would
#      match both NICs in lex-first order before `10-lan`/`10-uplink`).
#   2. `10-eth-dhcp.matchConfig.MACAddress` MUST equal the sentinel
#      `"00:00:00:00:00:00"` set by `mkForce` in net.nix:55-57. This
#      pins the neutralization mechanism (W5fu2 M1): a future patch
#      that drops the MAC sentinel — e.g. replacing it with a `Name`
#      match — would silently re-enable the catch-all on any
#      mac-less link and this test would catch it.
#   3. `10-uplink.addresses[0].Address` MUST equal the env's
#      `netUplinkIp/uplinkMask` (proves the per-MAC config applies
#      and the manifest's IP derivation is intact).
#   4. `10-lan.addresses[0].Address` MUST equal the env's
#      `netLanIp/lanMask` (W5fu2 M1: the per-MAC LAN config is the
#      other half of the static-config invariant; without it the
#      auto-NATed env would not have a default gateway).

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/net-vm-network-eval.sh"

# Env shape under test: lanSubnet=10.20.0.0/24 (net VM → 10.20.0.1/24),
# uplinkSubnet=192.0.2.0/30 (host=.1, net VM=.2 → 192.0.2.2/30). Keep
# in sync with the expected-address checks below.
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
      })
    ];
  };
  netVm = nixos.config.microvm.vms.sys-work-net.config;
  ed = netVm.config.systemd.network.networks."10-eth-dhcp";
  up = netVm.config.systemd.network.networks."10-uplink";
  lan = netVm.config.systemd.network.networks."10-lan";
in {
  ethDhcpMatchType = ed.matchConfig.Type or null;
  ethDhcpMatchMac  = ed.matchConfig.MACAddress or null;
  uplinkAddress = (builtins.head up.addresses).Address or "";
  lanAddress    = (builtins.head lan.addresses).Address or "";
}
EOF
)

OUT=$(nix-instantiate --eval --strict --json --expr "$EXPR" 2>/dev/null) || \
  fail "eval failed; cannot inspect net VM network config"

MATCH_TYPE=$(printf '%s' "$OUT" | jq -r '.ethDhcpMatchType // "null"')
MATCH_MAC=$(printf '%s'  "$OUT" | jq -r '.ethDhcpMatchMac  // "null"')
UPLINK_ADDR=$(printf '%s' "$OUT" | jq -r '.uplinkAddress')
LAN_ADDR=$(printf '%s'    "$OUT" | jq -r '.lanAddress')

if [ "$MATCH_TYPE" = "ether" ]; then
  fail "net VM '10-eth-dhcp' still has matchConfig.Type=ether (would DHCP both NICs lex-first vs 10-lan/10-uplink). See W5 H1."
fi
ok "net VM '10-eth-dhcp' is neutralized (matchConfig.Type = $MATCH_TYPE)"

# W5fu2 M1: pin the neutralization mechanism explicitly. The
# `mkForce` in net.nix:55-57 replaces the entire 10-eth-dhcp
# attrset with `matchConfig.MACAddress = "00:00:00:00:00:00"`. If a
# future patch swaps the sentinel for a different match key (e.g.
# Name=lo, or a real MAC), this assertion will catch it before any
# silently re-enabled catch-all hits a real interface.
if [ "$MATCH_MAC" != "00:00:00:00:00:00" ]; then
  fail "net VM '10-eth-dhcp' matchConfig.MACAddress is '$MATCH_MAC'; expected the all-zero sentinel '00:00:00:00:00:00' (see nixos-modules/net.nix:55-57)."
fi
ok "net VM '10-eth-dhcp' carries the sentinel MAC ($MATCH_MAC)"

if [ "$UPLINK_ADDR" != "192.0.2.2/30" ]; then
  fail "net VM '10-uplink' address is '$UPLINK_ADDR'; expected 192.0.2.2/30"
fi
ok "net VM '10-uplink' carries the env's static uplink address ($UPLINK_ADDR)"

# W5fu2 M1: the LAN-side static address is the other half of the
# per-MAC config. The env's netLanIp is `<lanSubnet base>.1` so for
# lanSubnet=10.20.0.0/24 this must be 10.20.0.1/24.
if [ "$LAN_ADDR" != "10.20.0.1/24" ]; then
  fail "net VM '10-lan' address is '$LAN_ADDR'; expected 10.20.0.1/24 (env work, lanSubnet=10.20.0.0/24)."
fi
ok "net VM '10-lan' carries the env's static LAN gateway address ($LAN_ADDR)"

log "==> net-vm-network-eval OK"
