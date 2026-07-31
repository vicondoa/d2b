{ config, lib, pkgs, ... }:

let
  cfg = config.d2bNetworkLocalNetVm;
  uplinkMac = "02:00:00:00:00:01";
  lanMac = "02:00:00:00:00:02";
in
{
  options.d2bNetworkLocalNetVm = {
    controllerUid = lib.mkOption {
      type = lib.types.int;
      internal = true;
      description = "Private fixed controller UID shared with the Host prerequisite.";
    };
    controllerGid = lib.mkOption {
      type = lib.types.int;
      internal = true;
      description = "Private fixed controller GID shared with the Host prerequisite.";
    };
    guestAgentPackage = lib.mkOption {
      type = lib.types.package;
      description = "Generic network guest-agent package.";
    };
  };

  config = {
    users.mutableUsers = false;
    users.groups.net-local-controller.gid = cfg.controllerGid;
    users.users.net-local-controller = {
      uid = cfg.controllerUid;
      isSystemUser = true;
      group = "net-local-controller";
      home = "/var/empty";
      shell = "${pkgs.shadow}/bin/nologin";
    };

    environment.systemPackages = [ cfg.guestAgentPackage ];

    networking.useNetworkd = true;
    networking.firewall.enable = false;
    systemd.network.networks = {
      "10-eth-dhcp" = lib.mkForce {
        matchConfig.MACAddress = "00:00:00:00:00:00";
      };
      "20-d2b-uplink" = {
        matchConfig.MACAddress = uplinkMac;
        networkConfig = {
          DHCP = "no";
          IPv6AcceptRA = false;
          LinkLocalAddressing = "no";
        };
        linkConfig.RequiredForOnline = "no";
      };
      "20-d2b-lan" = {
        matchConfig.MACAddress = lanMac;
        networkConfig = {
          DHCP = "no";
          IPv6AcceptRA = false;
          LinkLocalAddressing = "no";
        };
        linkConfig.RequiredForOnline = "no";
      };
    };
    systemd.network.links = {
      "20-d2b-uplink" = {
        matchConfig.MACAddress = uplinkMac;
        linkConfig.Name = "eth0";
      };
      "20-d2b-lan" = {
        matchConfig.MACAddress = lanMac;
        linkConfig.Name = "eth1";
      };
    };

    boot.kernel.sysctl = {
      "net.ipv4.ip_forward" = 1;
      "net.ipv4.conf.all.forwarding" = 1;
      "net.ipv6.conf.eth0.disable_ipv6" = 1;
      "net.ipv6.conf.eth0.accept_ra" = 0;
      "net.ipv6.conf.eth0.autoconf" = 0;
      "net.ipv6.conf.eth1.disable_ipv6" = 1;
      "net.ipv6.conf.eth1.accept_ra" = 0;
      "net.ipv6.conf.eth1.autoconf" = 0;
    };

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
  };
}
