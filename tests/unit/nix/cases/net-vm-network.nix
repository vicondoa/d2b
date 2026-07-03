# nix-unit cases migrated from tests/net-vm-network-eval.sh.
#
# Reconstructs the bash gate's work/safe/observability topology against the
# root d2b module set via `mkEval`, then asserts the net-VM/firewall
# contract directly from the rendered host config and composed per-VM configs.
#
# Spec correction (existing code is canon): the legacy bash gate read guest
# networkd/nftables details from `config.microvm.vms.<vm>.config`, which is a
# raw compatibility-shaped surface in the daemon-only tree and now lacks the
# realized guest networkd details. Current code stores the composed VM NixOS
# evaluations under `config.d2b._computed.<vm>.config`; these cases assert
# the same intended values there instead of preserving the bash gate's late
# skip after only the catch-all DHCP neutralization check.
{ mkEval, lib, ... }:

let
  fixture = { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };

    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
      allowUnsafeEastWest = true;
    };

    # Auto-declares env "obs" (lanSubnet 10.40.0.0/24, uplinkSubnet
    # 203.0.113.0/30), the sys-obs workload VM, and sys-obs-net.
    d2b.observability.enable = true;

    d2b.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
      mtu = 1280;
      mssClamp = true;
      lan.allowEastWest = true;
      homeLan = {
        enable = true;
        bridge = "br-home";
        address = "192.168.50.2/24";
        egress.allowCidrs = [ "192.168.50.53/32" ];
        portForwards = [
          { vm = "corp-vm"; port = 8443; protocol = "tcp"; }
          { vm = "corp-vm"; port = 51820; protocol = "udp"; }
        ];
        mdns = {
          enable = true;
          dnsmasqLocal.enable = true;
        };
      };
    };
    d2b.envs.safe = {
      lanSubnet = "10.30.0.0/24";
      uplinkSubnet = "198.51.100.0/30";
    };
    d2b.envs.home = {
      lanSubnet = "10.50.0.0/24";
      uplinkSubnet = "203.0.113.4/30";
      homeLan = {
        enable = true;
        bridge = "br-home";
        address = "192.168.50.3/24";
      };
    };

    d2b.vms.corp-vm = {
      enable = true;
      env = "work";
      index = 10;
      ssh.user = "alice";
      config = {
        networking.hostName = lib.mkDefault "corp-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
  };

  cfg = (mkEval [ fixture ]).config;
  computed = cfg.d2b._computed;

  workNet = computed.sys-work-net.config;
  safeNet = computed.sys-safe-net.config;
  homeOnlyNet = computed.sys-home-net.config;
  obsNet = computed.sys-obs-net.config;
  workGuest = computed.corp-vm.config;

  workEthDhcp = workNet.systemd.network.networks."10-eth-dhcp";
  workUplink = workNet.systemd.network.networks."10-uplink";
  workLan = workNet.systemd.network.networks."10-lan";
  workHome = workNet.systemd.network.networks."10-home";
  workHomeLink = workNet.systemd.network.links."10-home";
  workGuestDhcp = workGuest.systemd.network.networks."10-eth-dhcp";

  hostBrUp = cfg.systemd.network.networks."20-br-work-up";
  hostBrUpRoute = builtins.head hostBrUp.routes;
  hostBrLan = cfg.systemd.network.networks."20-br-work-lan";
  hostUpTap = cfg.systemd.network.networks."30-up-work";
  hostNetLanTap = cfg.systemd.network.networks."25-net-lan-work";
  workLanBridge = cfg.systemd.network.networks."30-lan-work";
  safeLanBridge = cfg.systemd.network.networks."30-lan-safe";
  obsLanBridge = cfg.systemd.network.networks."30-lan-obs";

  obsUplink = obsNet.systemd.network.networks."10-uplink";
  obsLan = obsNet.systemd.network.networks."10-lan";

  workRuleset = workNet.networking.nftables.ruleset;
  safeRuleset = safeNet.networking.nftables.ruleset;
  obsRuleset = obsNet.networking.nftables.ruleset;
  homeOnlyRuleset = homeOnlyNet.networking.nftables.ruleset;

  mssClampRule = "tcp flags syn tcp option maxseg size set rt mtu";
  lanToLanForwardRule = ''iifname "eth1" oifname "eth1" ct state new accept'';
  lanToUplinkAcceptRule = ''iifname "eth1" oifname "eth0" ct state new accept'';
  homeLanEgressAllowRule = ''iifname "eth1" oifname "home0" ip daddr 192.168.50.53/32 ct state new accept'';
  homeLanTcpDnatRule = ''iifname "home0" tcp dport 8443 dnat to 10.20.0.10:8443'';
  homeLanTcpForwardRule = ''iifname "home0" oifname "eth1" ip daddr 10.20.0.10 tcp dport 8443 ct state new accept'';
  homeLanUdpDnatRule = ''iifname "home0" udp dport 51820 dnat to 10.20.0.10:51820'';
  homeLanUdpForwardRule = ''iifname "home0" oifname "eth1" ip daddr 10.20.0.10 udp dport 51820 ct state new accept'';
  mdnsRule = ''udp dport 5353 ip daddr 224.0.0.251 accept'';
  dnsmasqLocalForward = "/local/224.0.0.251#5353";

  hasRule = ruleset: needle: lib.hasInfix needle ruleset;
  hasUdp5353 = ports: builtins.elem 5353 ports;

  lineOf = ruleset: needle:
    let
      indexed = lib.imap0
        (i: line: { lineNo = i + 1; inherit line; })
        (lib.splitString "\n" ruleset);
      matches = builtins.filter (entry: lib.hasInfix needle entry.line) indexed;
    in
    if matches == [ ] then null else (builtins.head matches).lineNo;

  beforeRule = ruleset: first: second:
    let
      firstLine = lineOf ruleset first;
      secondLine = lineOf ruleset second;
    in
    firstLine != null && secondLine != null && firstLine < secondLine;
in
{
  "net-vm-network/eth-dhcp-match-type-not-ether" = {
    expr = (workEthDhcp.matchConfig.Type or null) == "ether";
    expected = false;
  };
  "net-vm-network/eth-dhcp-match-mac-sentinel" = {
    expr = workEthDhcp.matchConfig.MACAddress or null;
    expected = "00:00:00:00:00:00";
  };

  # ---- work net VM static addressing + MTU propagation ----------------
  "net-vm-network/work-uplink-address" = {
    expr = (builtins.head workUplink.addresses).Address or "";
    expected = "192.0.2.2/30";
  };
  "net-vm-network/work-lan-address" = {
    expr = (builtins.head workLan.addresses).Address or "";
    expected = "10.20.0.1/24";
  };
  "net-vm-network/work-uplink-mtu" = {
    expr = workUplink.linkConfig.MTUBytes or null;
    expected = "1280";
  };
  "net-vm-network/work-lan-mtu" = {
    expr = workLan.linkConfig.MTUBytes or null;
    expected = "1280";
  };
  "net-vm-network/host-uplink-bridge-mtu" = {
    expr = hostBrUp.linkConfig.MTUBytes or null;
    expected = "1280";
  };
  "net-vm-network/host-lan-bridge-mtu" = {
    expr = hostBrLan.linkConfig.MTUBytes or null;
    expected = "1280";
  };
  "net-vm-network/host-uplink-tap-mtu" = {
    expr = hostUpTap.linkConfig.MTUBytes or null;
    expected = "1280";
  };
  "net-vm-network/host-net-lan-tap-mtu" = {
    expr = hostNetLanTap.linkConfig.MTUBytes or null;
    expected = "1280";
  };
  "net-vm-network/host-workload-lan-tap-mtu" = {
    expr = workLanBridge.linkConfig.MTUBytes or null;
    expected = "1280";
  };
  "net-vm-network/workload-guest-dhcp-mtu" = {
    expr = workGuestDhcp.linkConfig.MTUBytes or null;
    expected = "1280";
  };

  # ---- home-LAN opt-in surface ----------------------------------------
  "net-vm-network/safe-home-lan-default-off" = {
    expr = {
      optionDefault = cfg.d2b.envs.safe.homeLan.enable;
      hasGuestNetwork = builtins.hasAttr "10-home" safeNet.systemd.network.networks;
      hasGuestLink = builtins.hasAttr "10-home" safeNet.systemd.network.links;
      interfaceCount = builtins.length safeNet.microvm.interfaces;
    };
    expected = {
      optionDefault = false;
      hasGuestNetwork = false;
      hasGuestLink = false;
      interfaceCount = 2;
    };
  };
  "net-vm-network/obs-home-lan-default-off" = {
    expr = {
      optionDefault = cfg.d2b.envs.obs.homeLan.enable;
      hasGuestNetwork = builtins.hasAttr "10-home" obsNet.systemd.network.networks;
      hasGuestLink = builtins.hasAttr "10-home" obsNet.systemd.network.links;
      interfaceCount = builtins.length obsNet.microvm.interfaces;
    };
    expected = {
      optionDefault = false;
      hasGuestNetwork = false;
      hasGuestLink = false;
      interfaceCount = 2;
    };
  };
  "net-vm-network/work-home-lan-third-nic" = {
    expr = builtins.elem {
      type = "bridge";
      id = "work-h0";
      mac = cfg.d2b._index.envMeta.work.homeLan.mac;
      bridge = "br-home";
    } workNet.microvm.interfaces;
    expected = true;
  };
  "net-vm-network/work-home-lan-address" = {
    expr = (builtins.head workHome.addresses).Address or "";
    expected = "192.168.50.2/24";
  };
  "net-vm-network/work-home-lan-link-name" = {
    expr = {
      matchMac = workHome.matchConfig.MACAddress or "";
      linkMac = workHomeLink.matchConfig.MACAddress or "";
      name = workHomeLink.linkConfig.Name or "";
    };
    expected = {
      matchMac = cfg.d2b._index.envMeta.work.homeLan.mac;
      linkMac = cfg.d2b._index.envMeta.work.homeLan.mac;
      name = "home0";
    };
  };
  "net-vm-network/home-lan-egress-allow-before-host-blocklist" = {
    expr = beforeRule workRuleset homeLanEgressAllowRule "ip daddr 192.168.0.0/16 drop";
    expected = true;
  };
  "net-vm-network/safe-home-lan-egress-allow-absent" = {
    expr = hasRule safeRuleset homeLanEgressAllowRule;
    expected = false;
  };
  "net-vm-network/home-lan-port-forward-dnat-present" = {
    expr = {
      tcpDnat = hasRule workRuleset homeLanTcpDnatRule;
      tcpForward = hasRule workRuleset homeLanTcpForwardRule;
      udpDnat = hasRule workRuleset homeLanUdpDnatRule;
      udpForward = hasRule workRuleset homeLanUdpForwardRule;
      tcpDoesNotMatchUdpTuple = hasRule workRuleset ''iifname "home0" udp dport 8443 dnat'';
      udpDoesNotMatchTcpTuple = hasRule workRuleset ''iifname "home0" tcp dport 51820 dnat'';
    };
    expected = {
      tcpDnat = true;
      tcpForward = true;
      udpDnat = true;
      udpForward = true;
      tcpDoesNotMatchUdpTuple = false;
      udpDoesNotMatchTcpTuple = false;
    };
  };
  "net-vm-network/home-lan-port-forward-default-absent" = {
    expr = {
      safeTcpDnat = hasRule safeRuleset homeLanTcpDnatRule;
      safeTcpForward = hasRule safeRuleset homeLanTcpForwardRule;
      homeOnlyTcpDnat = hasRule homeOnlyRuleset homeLanTcpDnatRule;
      homeOnlyTcpForward = hasRule homeOnlyRuleset homeLanTcpForwardRule;
    };
    expected = {
      safeTcpDnat = false;
      safeTcpForward = false;
      homeOnlyTcpDnat = false;
      homeOnlyTcpForward = false;
    };
  };
  "net-vm-network/home-lan-mdns-avahi-present-only-when-enabled" = {
    expr = {
      workAvahi = {
        enable = workNet.services.avahi.enable or false;
        reflector = workNet.services.avahi.reflector or false;
        allowInterfaces = workNet.services.avahi.allowInterfaces or [ ];
      };
      homeOnlyAvahiEnable = homeOnlyNet.services.avahi.enable or false;
      safeAvahiEnable = safeNet.services.avahi.enable or false;
      workMdnsRule = hasRule workRuleset mdnsRule;
      homeOnlyMdnsRule = hasRule homeOnlyRuleset mdnsRule;
      safeMdnsRule = hasRule safeRuleset mdnsRule;
    };
    expected = {
      workAvahi = {
        enable = true;
        reflector = true;
        allowInterfaces = [ "eth1" "home0" ];
      };
      homeOnlyAvahiEnable = false;
      safeAvahiEnable = false;
      workMdnsRule = true;
      homeOnlyMdnsRule = false;
      safeMdnsRule = false;
    };
  };
  "net-vm-network/home-lan-dnsmasq-local-forwarding-gated" = {
    expr = {
      work = builtins.elem dnsmasqLocalForward workNet.services.dnsmasq.settings.server;
      homeOnly = builtins.elem dnsmasqLocalForward homeOnlyNet.services.dnsmasq.settings.server;
      safe = builtins.elem dnsmasqLocalForward safeNet.services.dnsmasq.settings.server;
    };
    expected = {
      work = true;
      homeOnly = false;
      safe = false;
    };
  };
  "net-vm-network/host-avahi-and-udp5353-not-opened" = {
    expr = {
      hostAvahi = cfg.services.avahi.enable or false;
      globalUdp5353 = hasUdp5353 cfg.networking.firewall.allowedUDPPorts;
      workLanUdp5353 = hasUdp5353 (cfg.networking.firewall.interfaces.${cfg.d2b._index.envMeta.work.lanBridge}.allowedUDPPorts or [ ]);
      workUplinkUdp5353 = hasUdp5353 (cfg.networking.firewall.interfaces.${cfg.d2b._index.envMeta.work.uplinkBridge}.allowedUDPPorts or [ ]);
      safeLanUdp5353 = hasUdp5353 (cfg.networking.firewall.interfaces.${cfg.d2b._index.envMeta.safe.lanBridge}.allowedUDPPorts or [ ]);
      safeUplinkUdp5353 = hasUdp5353 (cfg.networking.firewall.interfaces.${cfg.d2b._index.envMeta.safe.uplinkBridge}.allowedUDPPorts or [ ]);
    };
    expected = {
      hostAvahi = false;
      globalUdp5353 = false;
      workLanUdp5353 = false;
      workUplinkUdp5353 = false;
      safeLanUdp5353 = false;
      safeUplinkUdp5353 = false;
    };
  };

  # ---- nftables MSS clamp and inter-env/host drops ---------------------
  "net-vm-network/work-nft-mss-clamp-present" = {
    expr = hasRule workRuleset mssClampRule;
    expected = true;
  };
  "net-vm-network/safe-nft-mss-clamp-absent" = {
    expr = hasRule safeRuleset mssClampRule;
    expected = false;
  };
  "net-vm-network/work-nft-host-uplink-drop-present" = {
    expr = hasRule workRuleset "ip daddr 192.0.2.1 drop";
    expected = true;
  };
  "net-vm-network/work-nft-safe-lan-drop-present" = {
    expr = hasRule workRuleset "ip daddr 10.30.0.0/24 drop";
    expected = true;
  };
  "net-vm-network/work-nft-safe-uplink-drop-present" = {
    expr = hasRule workRuleset "ip daddr 198.51.100.0/30 drop";
    expected = true;
  };
  "net-vm-network/work-nft-obs-lan-drop-present" = {
    expr = hasRule workRuleset "ip daddr 10.40.0.0/24 drop";
    expected = true;
  };
  "net-vm-network/work-nft-obs-uplink-drop-present" = {
    expr = hasRule workRuleset "ip daddr 203.0.113.0/30 drop";
    expected = true;
  };
  "net-vm-network/safe-nft-work-lan-drop-present" = {
    expr = hasRule safeRuleset "ip daddr 10.20.0.0/24 drop";
    expected = true;
  };
  "net-vm-network/safe-nft-work-uplink-drop-present" = {
    expr = hasRule safeRuleset "ip daddr 192.0.2.0/30 drop";
    expected = true;
  };
  "net-vm-network/safe-nft-obs-lan-drop-present" = {
    expr = hasRule safeRuleset "ip daddr 10.40.0.0/24 drop";
    expected = true;
  };
  "net-vm-network/safe-nft-obs-uplink-drop-present" = {
    expr = hasRule safeRuleset "ip daddr 203.0.113.0/30 drop";
    expected = true;
  };
  "net-vm-network/work-nft-egress-accept-present" = {
    expr = hasRule workRuleset lanToUplinkAcceptRule;
    expected = true;
  };
  "net-vm-network/safe-nft-egress-accept-present" = {
    expr = hasRule safeRuleset lanToUplinkAcceptRule;
    expected = true;
  };
  "net-vm-network/work-nft-host-uplink-drop-before-egress-accept" = {
    expr = beforeRule workRuleset "ip daddr 192.0.2.1 drop" lanToUplinkAcceptRule;
    expected = true;
  };
  "net-vm-network/work-nft-safe-lan-drop-before-egress-accept" = {
    expr = beforeRule workRuleset "ip daddr 10.30.0.0/24 drop" lanToUplinkAcceptRule;
    expected = true;
  };
  "net-vm-network/work-nft-safe-uplink-drop-before-egress-accept" = {
    expr = beforeRule workRuleset "ip daddr 198.51.100.0/30 drop" lanToUplinkAcceptRule;
    expected = true;
  };
  "net-vm-network/safe-nft-work-lan-drop-before-egress-accept" = {
    expr = beforeRule safeRuleset "ip daddr 10.20.0.0/24 drop" lanToUplinkAcceptRule;
    expected = true;
  };
  "net-vm-network/safe-nft-work-uplink-drop-before-egress-accept" = {
    expr = beforeRule safeRuleset "ip daddr 192.0.2.0/30 drop" lanToUplinkAcceptRule;
    expected = true;
  };

  # ---- east-west positive and negative controls ------------------------
  "net-vm-network/work-lan-bridge-east-west-unisolated" = {
    expr = workLanBridge.bridgeConfig.Isolated or null;
    expected = false;
  };
  "net-vm-network/work-nft-lan-to-lan-forward-present" = {
    expr = hasRule workRuleset lanToLanForwardRule;
    expected = true;
  };
  "net-vm-network/safe-lan-bridge-isolated-default" = {
    expr = safeLanBridge.bridgeConfig.Isolated or null;
    expected = true;
  };
  "net-vm-network/safe-nft-lan-to-lan-forward-absent" = {
    expr = hasRule safeRuleset lanToLanForwardRule;
    expected = false;
  };

  # ---- host-side uplink/LAN bridge and tap contracts -------------------
  "net-vm-network/host-uplink-bridge-configure-without-carrier" = {
    expr = hostBrUp.networkConfig.ConfigureWithoutCarrier or null;
    expected = true;
  };
  "net-vm-network/host-uplink-bridge-link-local-addressing-disabled" = {
    expr = hostBrUp.networkConfig.LinkLocalAddressing or null;
    expected = "no";
  };
  "net-vm-network/host-lan-bridge-link-local-addressing-disabled" = {
    expr = hostBrLan.networkConfig.LinkLocalAddressing or null;
    expected = "no";
  };
  "net-vm-network/host-uplink-bridge-ipv6-ra-disabled" = {
    expr = hostBrUp.networkConfig.IPv6AcceptRA or null;
    expected = false;
  };
  "net-vm-network/host-lan-bridge-ipv6-ra-disabled" = {
    expr = hostBrLan.networkConfig.IPv6AcceptRA or null;
    expected = false;
  };
  "net-vm-network/host-uplink-bridge-route-destination" = {
    expr = hostBrUpRoute.Destination or null;
    expected = "10.20.0.0/24";
  };
  "net-vm-network/host-uplink-bridge-route-gateway" = {
    expr = hostBrUpRoute.Gateway or null;
    expected = "192.0.2.2";
  };
  "net-vm-network/net-lan-tap-bridge" = {
    expr = hostNetLanTap.networkConfig.Bridge or null;
    expected = "br-work-lan";
  };
  "net-vm-network/net-lan-tap-isolation-unset" = {
    expr = hostNetLanTap.bridgeConfig.Isolated or null;
    expected = null;
  };
  "net-vm-network/workload-lan-tap-bridge" = {
    expr = workLanBridge.networkConfig.Bridge or null;
    expected = "br-work-lan";
  };
  "net-vm-network/workload-lan-tap-isolation-east-west" = {
    expr = workLanBridge.bridgeConfig.Isolated or null;
    expected = false;
  };

  # ---- auto-declared observability env/net VM --------------------------
  "net-vm-network/obs-stack-vm-name" = {
    expr = cfg.d2b.observability.vmName;
    expected = "sys-obs";
  };
  "net-vm-network/obs-stack-vm-env" = {
    expr = (builtins.getAttr cfg.d2b.observability.vmName cfg.d2b.manifest).env or "";
    expected = "obs";
  };
  "net-vm-network/obs-uplink-address" = {
    expr = (builtins.head obsUplink.addresses).Address or "";
    expected = "203.0.113.2/30";
  };
  "net-vm-network/obs-lan-address" = {
    expr = (builtins.head obsLan.addresses).Address or "";
    expected = "10.40.0.1/24";
  };
  "net-vm-network/obs-nft-mss-clamp-absent" = {
    expr = hasRule obsRuleset mssClampRule;
    expected = false;
  };
  "net-vm-network/obs-nft-lan-to-lan-forward-absent" = {
    expr = hasRule obsRuleset lanToLanForwardRule;
    expected = false;
  };
  "net-vm-network/obs-lan-bridge-isolated-default" = {
    expr = obsLanBridge.bridgeConfig.Isolated or null;
    expected = true;
  };
  "net-vm-network/obs-nft-work-lan-drop-present" = {
    expr = hasRule obsRuleset "ip daddr 10.20.0.0/24 drop";
    expected = true;
  };
  "net-vm-network/obs-nft-work-uplink-drop-present" = {
    expr = hasRule obsRuleset "ip daddr 192.0.2.0/30 drop";
    expected = true;
  };
  "net-vm-network/obs-nft-safe-lan-drop-present" = {
    expr = hasRule obsRuleset "ip daddr 10.30.0.0/24 drop";
    expected = true;
  };
  "net-vm-network/obs-nft-safe-uplink-drop-present" = {
    expr = hasRule obsRuleset "ip daddr 198.51.100.0/30 drop";
    expected = true;
  };
  "net-vm-network/obs-nft-egress-accept-present" = {
    expr = hasRule obsRuleset lanToUplinkAcceptRule;
    expected = true;
  };
  "net-vm-network/obs-nft-work-lan-drop-before-egress-accept" = {
    expr = beforeRule obsRuleset "ip daddr 10.20.0.0/24 drop" lanToUplinkAcceptRule;
    expected = true;
  };
  "net-vm-network/obs-nft-work-uplink-drop-before-egress-accept" = {
    expr = beforeRule obsRuleset "ip daddr 192.0.2.0/30 drop" lanToUplinkAcceptRule;
    expected = true;
  };
  "net-vm-network/obs-nft-safe-lan-drop-before-egress-accept" = {
    expr = beforeRule obsRuleset "ip daddr 10.30.0.0/24 drop" lanToUplinkAcceptRule;
    expected = true;
  };
  "net-vm-network/obs-nft-safe-uplink-drop-before-egress-accept" = {
    expr = beforeRule obsRuleset "ip daddr 198.51.100.0/30 drop" lanToUplinkAcceptRule;
    expected = true;
  };
}
