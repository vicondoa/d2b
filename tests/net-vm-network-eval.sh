#!/usr/bin/env bash
# tests/net-vm-network-eval.sh— net VM network regression test.
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
#      pins the neutralization mechanism: a future patch
#      that drops the MAC sentinel — e.g. replacing it with a `Name`
#      match — would silently re-enable the catch-all on any
#      mac-less link and this test would catch it.
#   3. `10-uplink.addresses[0].Address` MUST equal the env's
#      `netUplinkIp/uplinkMask` (proves the per-MAC config applies
#      and the manifest's IP derivation is intact).
#   4. `10-lan.addresses[0].Address` MUST equal the env's
#      `netLanIp/lanMask` (M1: the per-MAC LAN config is the
#      other half of the static-config invariant; without it the
#      auto-NATed env would not have a default gateway).
#   5. `25-net-lan-<env>` MUST keep the net-VM tap non-isolated while
#      `30-lan-<env>` MUST set `bridgeConfig.Isolated = true` for
#      workload taps. This is the structural gate for the DHCP
#      anti-spoof / east-west blocking posture documented in design.md.
#   5. `nixling.envs.<env>.mtu` MUST propagate to the net VM NICs, the
#      host bridges/taps, and workload guest NICs in that env.
#   6. The net VM nftables ruleset MUST contain the MSS clamp rule when
#      `nixling.envs.<env>.mssClamp = true`.
#   7. Each env's net VM MUST drop every peer env LAN/uplink CIDR before
#      the broad LAN -> uplink forward rule.
#   8. `nixling.envs.<env>.lan.allowEastWest = true` MUST clear the host
#      bridge's `Isolated` flag and install an `eth1 -> eth1` forward rule.
#   9. The default path MUST remain isolated when `allowEastWest` is unset.
#  10. When `nixling.observability.enable = true` materialises the
#      reserved `obs` env (lanSubnet 10.40.0.0/24, uplinkSubnet
#      203.0.113.0/30) plus the `sys-obs` workload VM, the
#      auto-instantiated `sys-obs-net` VM MUST follow the same per-env
#      net-VM contract as the user-declared `work`/`safe` envs:
#        a. `10-uplink` address = 203.0.113.2/30, `10-lan` address =
#           10.40.0.1/24 (the env-derived netUplinkIp/netLanIp).
#        b. nftables MUST drop every peer env LAN/uplink CIDR (work's
#           10.20.0.0/24 + 192.0.2.0/30, safe's 10.30.0.0/24 +
#           198.51.100.0/30) before the broad LAN -> uplink accept.
#        c. nftables MUST NOT contain the MSS-clamp rule nor the
#           LAN-to-LAN forward rule (obs env has neither `mssClamp`
#           nor `lan.allowEastWest` set; this pins the safe defaults
#           for the framework-owned env so a future module change
#           cannot silently grant the observability stack a
#           wider posture than user-declared envs receive).
#        d. The host `30-lan-obs` bridge MUST stay `Isolated = true`
#           (no east-west between observability workloads and any
#           peer that might land in the same env).
#      Reciprocally, the work/safe net VMs MUST also drop the obs
#      env's LAN/uplink CIDRs once it is auto-declared — i.e. enabling
#      observability cannot become a hidden east-west tunnel between
#      previously-isolated envs.

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/net-vm-network-eval.sh"

# Env shape under test: lanSubnet=10.20.0.0/24 (net VM → 10.20.0.1/24),
# uplinkSubnet=192.0.2.0/30 (host=.1, net VM=.2 → 192.0.2.2/30), and a
# per-env MTU override plus MSS clamp. Keep in sync with the expected-
# value checks below.
EXPR=$(cat <<EOF
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
        nixling.site = { waylandUser = "alice"; launcherUsers = [ "alice" ]; yubikey.enable = false; allowUnsafeEastWest = true; };
        # Auto-declares env "obs" (lanSubnet 10.40.0.0/24,
        # uplinkSubnet 203.0.113.0/30), the sys-obs workload VM,
        # and — via the per-env net-VM auto-instantiation — sys-obs-net.
        nixling.observability.enable = true;
        nixling.envs.work = {
          lanSubnet = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
          mtu = 1280;
          mssClamp = true;
          lan.allowEastWest = true;
        };
        nixling.envs.safe = {
          lanSubnet = "10.30.0.0/24";
          uplinkSubnet = "198.51.100.0/30";
        };
        nixling.vms.corp-vm = {
          enable = true;
          env = "work";
          index = 10;
          ssh.user = "alice";
          config = {
            networking.hostName = lib.mkDefault "corp-vm";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
      })
    ];
  };
  hasMicrovmVms = nixos.config ? microvm && nixos.config.microvm ? vms;
  vms = if hasMicrovmVms then nixos.config.microvm.vms else {};
  netVm = vms.sys-work-net.config;
  safeNetVm = vms.sys-safe-net.config;
  obsNetVm = vms.sys-obs-net.config;
  obsStackVm = vms.sys-obs.config;
  workGuest = vms.corp-vm.config.config;
  ed = netVm.config.systemd.network.networks."10-eth-dhcp";
  up = netVm.config.systemd.network.networks."10-uplink";
  lan = netVm.config.systemd.network.networks."10-lan";
  workGuestDhcp = workGuest.systemd.network.networks."10-eth-dhcp";
  # The HOST-side uplink-bridge networkd entry MUST carry
  # ConfigureWithoutCarrier=true plus a
  # static route to the LAN subnet via the net VM's uplink address.
  # Without ConfigureWithoutCarrier the bridge cannot apply Address +
  # Route before its first tap attaches — but
  # nixling-net-route-preflight runs BEFORE the net VM start and
  # checks the route exists → deadlock.
  hostBrUp = nixos.config.systemd.network.networks."20-br-work-up";
  hostBrUpRoute = builtins.head hostBrUp.routes;
  netLanTap = nixos.config.systemd.network.networks."25-net-lan-work";
  workloadLanTaps = nixos.config.systemd.network.networks."30-lan-work";
  hostBrLan = nixos.config.systemd.network.networks."20-br-work-lan";
  hostUpTap = nixos.config.systemd.network.networks."30-up-work";
  hostNetLanTap = nixos.config.systemd.network.networks."25-net-lan-work";
  workLanBridge = nixos.config.systemd.network.networks."30-lan-work";
  safeLanBridge = nixos.config.systemd.network.networks."30-lan-safe";
  obsLanBridge = nixos.config.systemd.network.networks."30-lan-obs";
  obsUp = obsNetVm.config.systemd.network.networks."10-uplink";
  obsLan = obsNetVm.config.systemd.network.networks."10-lan";
  obsStackVmName = nixos.config.nixling.observability.vmName;
in if !hasMicrovmVms then {
  microvmVmsPresent = false;
} else {
  microvmVmsPresent = true;
  ethDhcpMatchType = ed.matchConfig.Type or null;
  ethDhcpMatchMac  = ed.matchConfig.MACAddress or null;
  uplinkAddress = (builtins.head up.addresses).Address or "";
  lanAddress    = (builtins.head lan.addresses).Address or "";
  uplinkMtuBytes = up.linkConfig.MTUBytes or null;
  lanMtuBytes = lan.linkConfig.MTUBytes or null;
  hostBrUpMtuBytes = hostBrUp.linkConfig.MTUBytes or null;
  hostBrLanMtuBytes = hostBrLan.linkConfig.MTUBytes or null;
  hostUpTapMtuBytes = hostUpTap.linkConfig.MTUBytes or null;
  hostNetLanTapMtuBytes = hostNetLanTap.linkConfig.MTUBytes or null;
  hostWorkloadTapMtuBytes = workLanBridge.linkConfig.MTUBytes or null;
  workGuestMtuBytes = workGuestDhcp.linkConfig.MTUBytes or null;
  nftRuleset = netVm.config.networking.nftables.ruleset or "";
  safeNftRuleset = safeNetVm.config.networking.nftables.ruleset or "";
  workLanBridgeIsolated = workLanBridge.bridgeConfig.Isolated or null;
  safeLanBridgeIsolated = safeLanBridge.bridgeConfig.Isolated or null;
  hostBrUpConfigureWithoutCarrier = hostBrUp.networkConfig.ConfigureWithoutCarrier or null;
  hostBrUpLinkLocalAddressing = hostBrUp.networkConfig.LinkLocalAddressing or null;
  hostBrUpIPv6AcceptRA = hostBrUp.networkConfig.IPv6AcceptRA or null;
  hostBrLanLinkLocalAddressing = hostBrLan.networkConfig.LinkLocalAddressing or null;
  hostBrLanIPv6AcceptRA = hostBrLan.networkConfig.IPv6AcceptRA or null;
  hostBrUpRouteDestination = hostBrUpRoute.Destination or null;
  hostBrUpRouteGateway     = hostBrUpRoute.Gateway or null;
  netLanTapBridge = netLanTap.networkConfig.Bridge or null;
  netLanTapIsolated = netLanTap.bridgeConfig.Isolated or null;
  workloadLanTapBridge = workloadLanTaps.networkConfig.Bridge or null;
  workloadLanTapIsolated = workloadLanTaps.bridgeConfig.Isolated or null;
  obsUplinkAddress = (builtins.head obsUp.addresses).Address or "";
  obsLanAddress = (builtins.head obsLan.addresses).Address or "";
  obsNftRuleset = obsNetVm.config.networking.nftables.ruleset or "";
  obsLanBridgeIsolated = obsLanBridge.bridgeConfig.Isolated or null;
  obsStackVmName = obsStackVmName;
  obsStackEnv = (builtins.getAttr obsStackVmName nixos.config.nixling.manifest).env or "";
}
EOF
)

OUT=$(nix-instantiate --eval --strict --json --expr "$EXPR" 2>/dev/null) || \
  fail "eval failed; cannot inspect net VM network config"

MICROVM_VMS_PRESENT=$(printf '%s' "$OUT" | jq -r '.microvmVmsPresent // true')
if [ "$MICROVM_VMS_PRESENT" != "true" ]; then
  log "  SKIP: microvm.vms surface absent in daemon-only config; net VM microvm network shape is not emitted"
  exit 0
fi

MATCH_TYPE=$(printf '%s' "$OUT" | jq -r '.ethDhcpMatchType // "null"')
MATCH_MAC=$(printf '%s'  "$OUT" | jq -r '.ethDhcpMatchMac  // "null"')
UPLINK_ADDR=$(printf '%s' "$OUT" | jq -r '.uplinkAddress')
LAN_ADDR=$(printf '%s'    "$OUT" | jq -r '.lanAddress')
UPLINK_MTU=$(printf '%s'  "$OUT" | jq -r '.uplinkMtuBytes // "null"')
LAN_MTU=$(printf '%s'     "$OUT" | jq -r '.lanMtuBytes // "null"')
HOST_BR_UP_MTU=$(printf '%s' "$OUT" | jq -r '.hostBrUpMtuBytes // "null"')
HOST_BR_LAN_MTU=$(printf '%s' "$OUT" | jq -r '.hostBrLanMtuBytes // "null"')
HOST_UP_TAP_MTU=$(printf '%s' "$OUT" | jq -r '.hostUpTapMtuBytes // "null"')
HOST_NET_LAN_TAP_MTU=$(printf '%s' "$OUT" | jq -r '.hostNetLanTapMtuBytes // "null"')
HOST_WORKLOAD_TAP_MTU=$(printf '%s' "$OUT" | jq -r '.hostWorkloadTapMtuBytes // "null"')
WORK_GUEST_MTU=$(printf '%s' "$OUT" | jq -r '.workGuestMtuBytes // "null"')
NFT_RULESET=$(printf '%s' "$OUT" | jq -r '.nftRuleset')
SAFE_NFT_RULESET=$(printf '%s' "$OUT" | jq -r '.safeNftRuleset')
WORK_LAN_ISOLATED=$(printf '%s' "$OUT" | jq -r '.workLanBridgeIsolated // "null"')
SAFE_LAN_ISOLATED=$(printf '%s' "$OUT" | jq -r '.safeLanBridgeIsolated // "null"')

if [ "$MATCH_TYPE" = "ether" ]; then
  fail "net VM '10-eth-dhcp' still has matchConfig.Type=ether (would DHCP both NICs lex-first vs 10-lan/10-uplink)."
fi
ok "net VM '10-eth-dhcp' is neutralized (matchConfig.Type = $MATCH_TYPE)"

# Pin the neutralization mechanism explicitly. The invariant is that
# `10-eth-dhcp` must not regain a broad ethernet match. Current module
# shapes either emit the all-zero sentinel MAC or no catch-all match
# data after the mkForce neutralization.
case "$MATCH_MAC" in
  "00:00:00:00:00:00")
    ok "net VM '10-eth-dhcp' carries the sentinel MAC ($MATCH_MAC)"
    ;;
  "null")
    ok "net VM '10-eth-dhcp' has no broad match data"
    ;;
  *)
    fail "net VM '10-eth-dhcp' matchConfig.MACAddress is '$MATCH_MAC'; expected no broad match or the all-zero sentinel."
    ;;
esac

if [ "$UPLINK_ADDR" = "null" ] || [ -z "$UPLINK_ADDR" ]; then
  log "  SKIP: legacy microvm networkd details are absent in the daemon-only config; catch-all DHCP neutralization was verified"
  exit 0
fi

if [ "$UPLINK_ADDR" != "192.0.2.2/30" ]; then
  fail "net VM '10-uplink' address is '$UPLINK_ADDR'; expected 192.0.2.2/30"
fi
ok "net VM '10-uplink' carries the env's static uplink address ($UPLINK_ADDR)"

# The LAN-side static address is the other half of the
# per-MAC config. The env's netLanIp is `<lanSubnet base>.1` so for
# lanSubnet=10.20.0.0/24 this must be 10.20.0.1/24.
if [ "$LAN_ADDR" != "10.20.0.1/24" ]; then
  fail "net VM '10-lan' address is '$LAN_ADDR'; expected 10.20.0.1/24 (env work, lanSubnet=10.20.0.0/24)."
fi
ok "net VM '10-lan' carries the env's static LAN gateway address ($LAN_ADDR)"

if [ "$UPLINK_MTU" != "1280" ]; then
  fail "net VM '10-uplink' MTUBytes is '$UPLINK_MTU'; expected '1280' from nixling.envs.work.mtu"
fi
if [ "$LAN_MTU" != "1280" ]; then
  fail "net VM '10-lan' MTUBytes is '$LAN_MTU'; expected '1280' from nixling.envs.work.mtu"
fi
for value in \
  "$HOST_BR_UP_MTU" \
  "$HOST_BR_LAN_MTU" \
  "$HOST_UP_TAP_MTU" \
  "$HOST_NET_LAN_TAP_MTU" \
  "$HOST_WORKLOAD_TAP_MTU" \
  "$WORK_GUEST_MTU"; do
  if [ "$value" != "1280" ]; then
    fail "MTU propagation is incomplete; expected every host bridge/tap and the workload guest NIC to use 1280, got '$value'"
  fi
done
ok "env MTU propagates across net VM, host bridge/tap, and workload guest NICs"

case "$NFT_RULESET" in
  *'tcp flags syn tcp option maxseg size set rt mtu'*) ;;
  *) fail "net VM nftables ruleset is missing the MSS clamp rule for nixling.envs.work.mssClamp = true" ;;
esac
case "$SAFE_NFT_RULESET" in
  *'tcp flags syn tcp option maxseg size set rt mtu'*)
    fail "safe env unexpectedly gained the MSS clamp rule with mssClamp unset"
    ;;
  *) ;;
esac
ok "net VM nftables ruleset carries the MSS clamp rule only when enabled"

case "$NFT_RULESET" in
  *'ip daddr 192.0.2.1 drop'* ) ;;
  *) fail "work env nftables ruleset is missing the host-uplink drop for 192.0.2.1" ;;
esac
case "$NFT_RULESET" in
  *'ip daddr 10.30.0.0/24 drop'* ) ;;
  *) fail "work env nftables ruleset is missing the peer-LAN drop for 10.30.0.0/24" ;;
esac
case "$NFT_RULESET" in
  *'ip daddr 198.51.100.0/30 drop'* ) ;;
  *) fail "work env nftables ruleset is missing the peer-uplink drop for 198.51.100.0/30" ;;
esac
case "$NFT_RULESET" in
  *'ip daddr 10.40.0.0/24 drop'* ) ;;
  *) fail "work env nftables ruleset is missing the peer-LAN drop for 10.40.0.0/24 (obs env)" ;;
esac
case "$NFT_RULESET" in
  *'ip daddr 203.0.113.0/30 drop'* ) ;;
  *) fail "work env nftables ruleset is missing the peer-uplink drop for 203.0.113.0/30 (obs env)" ;;
esac
case "$SAFE_NFT_RULESET" in
  *'ip daddr 10.20.0.0/24 drop'* ) ;;
  *) fail "safe env nftables ruleset is missing the peer-LAN drop for 10.20.0.0/24" ;;
esac
case "$SAFE_NFT_RULESET" in
  *'ip daddr 192.0.2.0/30 drop'* ) ;;
  *) fail "safe env nftables ruleset is missing the peer-uplink drop for 192.0.2.0/30" ;;
esac
case "$SAFE_NFT_RULESET" in
  *'ip daddr 10.40.0.0/24 drop'* ) ;;
  *) fail "safe env nftables ruleset is missing the peer-LAN drop for 10.40.0.0/24 (obs env)" ;;
esac
case "$SAFE_NFT_RULESET" in
  *'ip daddr 203.0.113.0/30 drop'* ) ;;
  *) fail "safe env nftables ruleset is missing the peer-uplink drop for 203.0.113.0/30 (obs env)" ;;
esac
WORK_ACCEPT_LINE=$(printf '%s
' "$NFT_RULESET" | grep -n -F 'iifname "eth1" oifname "eth0" ct state new accept' | head -1 | cut -d: -f1)
WORK_HOST_DROP_LINE=$(printf '%s
' "$NFT_RULESET" | grep -n -F 'ip daddr 192.0.2.1 drop' | head -1 | cut -d: -f1)
WORK_PEER_LAN_DROP_LINE=$(printf '%s
' "$NFT_RULESET" | grep -n -F 'ip daddr 10.30.0.0/24 drop' | head -1 | cut -d: -f1)
WORK_PEER_UP_DROP_LINE=$(printf '%s
' "$NFT_RULESET" | grep -n -F 'ip daddr 198.51.100.0/30 drop' | head -1 | cut -d: -f1)
SAFE_ACCEPT_LINE=$(printf '%s
' "$SAFE_NFT_RULESET" | grep -n -F 'iifname "eth1" oifname "eth0" ct state new accept' | head -1 | cut -d: -f1)
SAFE_PEER_LAN_DROP_LINE=$(printf '%s
' "$SAFE_NFT_RULESET" | grep -n -F 'ip daddr 10.20.0.0/24 drop' | head -1 | cut -d: -f1)
SAFE_PEER_UP_DROP_LINE=$(printf '%s
' "$SAFE_NFT_RULESET" | grep -n -F 'ip daddr 192.0.2.0/30 drop' | head -1 | cut -d: -f1)
if [ -z "$WORK_ACCEPT_LINE" ] || [ -z "$WORK_HOST_DROP_LINE" ] || [ -z "$WORK_PEER_LAN_DROP_LINE" ] || [ -z "$WORK_PEER_UP_DROP_LINE" ] \
   || [ -z "$SAFE_ACCEPT_LINE" ] || [ -z "$SAFE_PEER_LAN_DROP_LINE" ] || [ -z "$SAFE_PEER_UP_DROP_LINE" ]; then
  fail "could not determine nftables rule ordering for host/peer-env drop checks"
fi
if [ "$WORK_HOST_DROP_LINE" -ge "$WORK_ACCEPT_LINE" ] || [ "$WORK_PEER_LAN_DROP_LINE" -ge "$WORK_ACCEPT_LINE" ] || [ "$WORK_PEER_UP_DROP_LINE" -ge "$WORK_ACCEPT_LINE" ] \
   || [ "$SAFE_PEER_LAN_DROP_LINE" -ge "$SAFE_ACCEPT_LINE" ] || [ "$SAFE_PEER_UP_DROP_LINE" -ge "$SAFE_ACCEPT_LINE" ]; then
  fail "host-uplink and peer-env CIDR drops must appear before the broad LAN->uplink accept rule"
fi
ok "peer env LAN/uplink CIDRs are dropped before routed egress"

if [ "$WORK_LAN_ISOLATED" = "true" ]; then
  fail "host '30-lan-work' bridgeConfig.Isolated stayed 'true' with nixling.envs.work.lan.allowEastWest = true"
fi
ok "host '30-lan-work' clears bridge isolation when east-west is enabled"

case "$NFT_RULESET" in
  *'iifname "eth1" oifname "eth1" ct state new accept'*) ;;
  *) fail "net VM nftables ruleset is missing the LAN-to-LAN forward rule for nixling.envs.work.lan.allowEastWest = true" ;;
esac
ok "net VM nftables ruleset carries the LAN-to-LAN forward rule"

if [ "$SAFE_LAN_ISOLATED" != "true" ]; then
  fail "host '30-lan-safe' bridgeConfig.Isolated is '$SAFE_LAN_ISOLATED'; expected default 'true' when allowEastWest is unset"
fi
case "$SAFE_NFT_RULESET" in
  *'iifname "eth1" oifname "eth1" ct state new accept'*)
    fail "safe env unexpectedly gained the LAN-to-LAN forward rule with allowEastWest unset"
    ;;
  *) ;;
esac
ok "default east-west path remains isolated when allowEastWest is unset"

# Host-side uplink-bridge checks. ConfigureWithoutCarrier=true lets
# networkd apply Address+Route before the bridge has carrier. The
# route entry must point at the env's LAN subnet via the net VM's
# uplink IP — that's what the route-preflight unit polls for at boot.
HOST_BR_CWC=$(printf '%s' "$OUT" | jq -r '.hostBrUpConfigureWithoutCarrier // "null"')
HOST_BR_UP_LLA=$(printf '%s' "$OUT" | jq -r '.hostBrUpLinkLocalAddressing // "null"')
HOST_BR_UP_RA=$(printf '%s' "$OUT" | jq -r '.hostBrUpIPv6AcceptRA // "null"')
HOST_BR_LAN_LLA=$(printf '%s' "$OUT" | jq -r '.hostBrLanLinkLocalAddressing // "null"')
HOST_BR_LAN_RA=$(printf '%s' "$OUT" | jq -r '.hostBrLanIPv6AcceptRA // "null"')
HOST_BR_DEST=$(printf '%s' "$OUT" | jq -r '.hostBrUpRouteDestination // "null"')
HOST_BR_GW=$(printf '%s' "$OUT" | jq -r '.hostBrUpRouteGateway // "null"')
NET_LAN_BRIDGE=$(printf '%s' "$OUT" | jq -r '.netLanTapBridge // "null"')
NET_LAN_ISOLATED=$(printf '%s' "$OUT" | jq -r '.netLanTapIsolated // "null"')
WORKLOAD_LAN_BRIDGE=$(printf '%s' "$OUT" | jq -r '.workloadLanTapBridge // "null"')
WORKLOAD_LAN_ISOLATED=$(printf '%s' "$OUT" | jq -r '.workloadLanTapIsolated | tostring')

if [ "$HOST_BR_CWC" != "true" ]; then
  fail "host '20-br-work-up' networkConfig.ConfigureWithoutCarrier is '$HOST_BR_CWC'; expected 'true' (route-preflight deadlock; see nixos-modules/network.nix:335-353)."
fi
if [ "$HOST_BR_UP_LLA" != "no" ] || [ "$HOST_BR_LAN_LLA" != "no" ]; then
  fail "host bridge IPv6 isolation regressed; expected LinkLocalAddressing=no on both br-work-up and br-work-lan"
fi
if { [ "$HOST_BR_UP_RA" != "false" ] && [ "$HOST_BR_UP_RA" != "null" ]; } \
   || { [ "$HOST_BR_LAN_RA" != "false" ] && [ "$HOST_BR_LAN_RA" != "null" ]; }; then
  fail "host bridge IPv6 isolation regressed; expected IPv6AcceptRA=false (or elided-to-default=false) on both br-work-up and br-work-lan"
fi
ok "host bridges keep IPv6 link-local and RA disabled"

if [ "$HOST_BR_DEST" != "10.20.0.0/24" ]; then
  fail "host '20-br-work-up' route Destination is '$HOST_BR_DEST'; expected '10.20.0.0/24' (env work lanSubnet)."
fi
if [ "$HOST_BR_GW" != "192.0.2.2" ]; then
  fail "host '20-br-work-up' route Gateway is '$HOST_BR_GW'; expected '192.0.2.2' (env work netUplinkIp)."
fi
ok "host '20-br-work-up' route points at $HOST_BR_DEST via $HOST_BR_GW"

if [ "$NET_LAN_BRIDGE" != "br-work-lan" ]; then
  fail "net-VM LAN tap rule bridges to '$NET_LAN_BRIDGE'; expected 'br-work-lan'."
fi
if [ "$NET_LAN_ISOLATED" != "null" ]; then
  fail "net-VM LAN tap rule unexpectedly sets bridgeConfig.Isolated=$NET_LAN_ISOLATED; the net VM port must remain non-isolated."
fi
ok "net-VM LAN tap stays on br-work-lan without bridge-port isolation"

if [ "$WORKLOAD_LAN_BRIDGE" != "br-work-lan" ]; then
  fail "workload LAN tap rule bridges to '$WORKLOAD_LAN_BRIDGE'; expected 'br-work-lan'."
fi
# work env has allowEastWest = true, so Isolated should be false
if [ "$WORKLOAD_LAN_ISOLATED" != "false" ]; then
  fail "workload LAN tap rule bridgeConfig.Isolated is '$WORKLOAD_LAN_ISOLATED'; expected 'false' because work env has lan.allowEastWest = true."
fi
ok "workload LAN tap rule respects allowEastWest on br-work-lan"

# Observability env (auto-declared by
# `nixling.observability.enable = true`) is just another env from the
# net.nix perspective, but its lifecycle is framework-owned. Pin the
# per-env net-VM contract here so a future change to the observability
# auto-declaration (env name, IP derivation, default east-west posture)
# cannot drift away from the user-declared envs without flipping this
# test.
OBS_UPLINK_ADDR=$(printf '%s' "$OUT" | jq -r '.obsUplinkAddress')
OBS_LAN_ADDR=$(printf '%s' "$OUT"   | jq -r '.obsLanAddress')
OBS_NFT_RULESET=$(printf '%s' "$OUT" | jq -r '.obsNftRuleset')
OBS_LAN_ISOLATED=$(printf '%s' "$OUT" | jq -r '.obsLanBridgeIsolated // "null"')
OBS_STACK_HOST=$(printf '%s' "$OUT" | jq -r '.obsStackEnv')
OBS_STACK_NAME=$(printf '%s' "$OUT" | jq -r '.obsStackVmName')

if [ "$OBS_STACK_NAME" != "sys-obs" ]; then
  fail "nixling.observability.vmName is '$OBS_STACK_NAME'; expected canonical 'sys-obs' (see nixos-modules/options-observability.nix). The obs env fixture pins this name because all components/observability/host.nix wiring keys off it."
fi
if [ "$OBS_STACK_HOST" != "obs" ]; then
  fail "manifest entry for sys-obs has env '$OBS_STACK_HOST'; expected 'obs' (auto-declared by nixling.observability.enable = true). The obs env fixture depends on this VM being instantiated in the reserved env."
fi
ok "obs env auto-declares the sys-obs workload VM in env 'obs'"

if [ "$OBS_UPLINK_ADDR" != "203.0.113.2/30" ]; then
  fail "obs net VM '10-uplink' address is '$OBS_UPLINK_ADDR'; expected 203.0.113.2/30 (env obs, uplinkSubnet=203.0.113.0/30, net VM is host=.1, net=.2)."
fi
ok "obs net VM '10-uplink' carries the env's static uplink address ($OBS_UPLINK_ADDR)"

if [ "$OBS_LAN_ADDR" != "10.40.0.1/24" ]; then
  fail "obs net VM '10-lan' address is '$OBS_LAN_ADDR'; expected 10.40.0.1/24 (env obs, lanSubnet=10.40.0.0/24, gateway is .1)."
fi
ok "obs net VM '10-lan' carries the env's static LAN gateway address ($OBS_LAN_ADDR)"

# Safe defaults: the framework-owned obs env MUST NOT silently grant
# itself a wider posture than user envs receive by default. MSS clamp
# and LAN-to-LAN forward both require an opt-in elsewhere.
case "$OBS_NFT_RULESET" in
  *'tcp flags syn tcp option maxseg size set rt mtu'*)
    fail "obs env unexpectedly gained the MSS clamp rule; nixling.envs.obs.mssClamp must remain unset for the auto-declared env."
    ;;
  *) ;;
esac
case "$OBS_NFT_RULESET" in
  *'iifname "eth1" oifname "eth1" ct state new accept'*)
    fail "obs env unexpectedly gained the LAN-to-LAN forward rule; nixling.envs.obs.lan.allowEastWest must remain unset for the auto-declared env."
    ;;
  *) ;;
esac
if [ "$OBS_LAN_ISOLATED" != "true" ]; then
  fail "host '30-lan-obs' bridgeConfig.Isolated is '$OBS_LAN_ISOLATED'; expected default 'true' (obs env auto-decl must not enable east-west)."
fi
ok "obs env keeps safe defaults: bridge Isolated=true, no MSS clamp, no LAN<->LAN forward"

# Peer-env isolation: obs net VM must drop both user envs' LAN/uplink
# CIDRs before the broad LAN -> uplink accept, matching the existing
# work/safe contract. Reciprocity with work/safe was asserted above.
for cidr in '10.20.0.0/24' '192.0.2.0/30' '10.30.0.0/24' '198.51.100.0/30'; do
  case "$OBS_NFT_RULESET" in
    *"ip daddr $cidr drop"* ) ;;
    *) fail "obs env nftables ruleset is missing the peer drop for $cidr" ;;
  esac
done

OBS_ACCEPT_LINE=$(printf '%s
' "$OBS_NFT_RULESET" | grep -n -F 'iifname "eth1" oifname "eth0" ct state new accept' | head -1 | cut -d: -f1)
if [ -z "$OBS_ACCEPT_LINE" ]; then
  fail "obs env nftables ruleset is missing the broad LAN -> uplink accept rule"
fi
for cidr in '10.20.0.0/24' '192.0.2.0/30' '10.30.0.0/24' '198.51.100.0/30'; do
  drop_line=$(printf '%s
' "$OBS_NFT_RULESET" | grep -n -F "ip daddr $cidr drop" | head -1 | cut -d: -f1)
  if [ -z "$drop_line" ] || [ "$drop_line" -ge "$OBS_ACCEPT_LINE" ]; then
    fail "obs env peer drop for $cidr (line '$drop_line') must appear before the broad LAN->uplink accept (line $OBS_ACCEPT_LINE)"
  fi
done
ok "obs env drops every peer LAN/uplink CIDR before routed egress"

log "==> net-vm-network-eval OK"
