# Generic Network Guest baseline.
#
# Desired CIDRs, DHCP, routes, firewall policy, interface identities, and
# attachment state are supplied by the committed Network resource through the
# Provider controller. This module contains only invariant Guest hardening.
{ lib, ... }:

{
  networking.useNetworkd = true;
  networking.firewall.enable = false;

  # A Guest may inherit the framework catch-all DHCP network from base.nix.
  # Network Guests have provider-owned interfaces and must never acquire an
  # accidental DHCP address before their committed Network projection binds.
  systemd.network.networks."10-eth-dhcp" = lib.mkForce {
    matchConfig.MACAddress = "00:00:00:00:00:00";
  };

  boot.kernel.sysctl = {
    "net.ipv4.ip_forward" = 1;
    "net.ipv4.conf.all.forwarding" = 1;
    "net.ipv6.conf.all.disable_ipv6" = 1;
    "net.ipv6.conf.all.accept_ra" = 0;
    "net.ipv6.conf.all.autoconf" = 0;
  };

  # Network Guests are IPv4-only. The Provider applies the per-link
  # suppression again after bridge/TAP realization.
  networking.nftables = {
    enable = true;
    ruleset = ''
      table ip6 filter {
        chain input { type filter hook input priority 0; policy drop; }
        chain forward { type filter hook forward priority 0; policy drop; }
        chain output { type filter hook output priority 0; policy drop; }
      }
    '';
  };
}
