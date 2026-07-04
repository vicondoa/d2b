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
      externalNetwork = {
        attachment = {
          enable = true;
          interface = "eno1";
        };
        egress = {
          enable = true;
          allowedCidrs = [ "192.168.1.0/24" ];
        };
        portForwards = [
          { protocol = "tcp"; listenPort = 2222; vm = "corp-vm"; targetPort = 22; }
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
    d2b.envs.quiet = {
      lanSubnet = "10.50.0.0/24";
      uplinkSubnet = "198.51.100.4/30";
      externalNetwork = {
        attachment = {
          enable = true;
          interface = "eno2";
        };
        mdns = {
          enable = true;
          reflector.enable = false;
        };
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
  quietNet = computed.sys-quiet-net.config;
  obsNet = computed.sys-obs-net.config;
  workGuest = computed.corp-vm.config;

  workEthDhcp = workNet.systemd.network.networks."10-eth-dhcp";
  workUplink = workNet.systemd.network.networks."10-uplink";
  workLan = workNet.systemd.network.networks."10-lan";
  workHome = workNet.systemd.network.networks."10-home";
  workHomeLink = workNet.systemd.network.links."10-home";
  workGuestDhcp = workGuest.systemd.network.networks."10-eth-dhcp";
  workHomeIface = builtins.elemAt workNet.microvm.interfaces 2;
  workHomeMac = workHomeIface.mac;

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
  workDnsmasqServers = workNet.services.dnsmasq.settings.server;

  mssClampRule = "tcp flags syn tcp option maxseg size set rt mtu";
  lanToLanForwardRule = ''iifname "eth1" oifname "eth1" ct state new accept'';
  lanToUplinkAcceptRule = ''iifname "eth1" oifname "eth0" ct state new accept'';
  lanToHomeAcceptRule = ''iifname "eth1" oifname "external0" ip daddr 192.168.1.0/24 ct state new accept'';
  externalNetworkEth0DropRule = ''iifname "eth1" oifname "eth0" ip daddr 192.168.1.0/24 drop'';
  homeDnatRule = ''iifname "external0" tcp dport 2222 dnat to 10.20.0.10:22'';
  homeForwardRule = ''iifname "external0" oifname "eth1" ip daddr 10.20.0.10 tcp dport 22 ct state new accept'';
  homeMasqueradeRule = ''oifname "external0" masquerade'';

  hostJson = builtins.fromJSON cfg.d2b._bundle.hostJson.jsonText;
  workHostEnv = builtins.head (builtins.filter (env: env.env == "work") hostJson.environments);
  processesJson = cfg.d2b._bundle.processesJson.data;
  workNetDag = builtins.head (builtins.filter (dag: dag.vm == "sys-work-net") processesJson.vms);
  workNetChNode = builtins.head (builtins.filter (node: node.id == "cloud-hypervisor") workNetDag.nodes);
  workNetHomeProcessIface = builtins.elemAt workNetChNode.networkInterfaces 2;
  workNetHomeArgvFd =
    lib.any
      (arg: lib.hasInfix "fd=10" arg && lib.hasInfix "mac=${workHomeMac}" arg)
      workNetChNode.argv;
  mdnsHomeInputRule = ''iifname "external0" udp dport 5353 accept'';
  mdnsLanInputRule = ''iifname "eth1" udp dport 5353 accept'';

  hasRule = ruleset: needle: lib.hasInfix needle ruleset;

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

  # ---- external network net VM attachment --------------------------------------
  "net-vm-network/home-lan-net-vm-third-interface-type" = {
    expr = workHomeIface.type;
    expected = "macvtap";
  };
  "net-vm-network/home-lan-net-vm-third-interface-id" = {
    expr = workHomeIface.id;
    expected = "work-h0";
  };
  "net-vm-network/home-lan-net-vm-macvtap-parent" = {
    expr = workHomeIface.macvtap.link;
    expected = "eno1";
  };
  "net-vm-network/home-lan-net-vm-macvtap-mode" = {
    expr = workHomeIface.macvtap.mode;
    expected = "bridge";
  };
  "net-vm-network/home-lan-guest-renamed-external0" = {
    expr = workHomeLink.linkConfig.Name or null;
    expected = "external0";
  };
  "net-vm-network/home-lan-guest-dhcp" = {
    expr = workHome.networkConfig.DHCP or null;
    expected = "ipv4";
  };
  "net-vm-network/home-lan-guest-dhcp-routes-ignored" = {
    expr = workHome.dhcpV4Config.UseRoutes or null;
    expected = false;
  };
  "net-vm-network/home-lan-host-nm-unmanaged-macvtap-only" = {
    expr = {
      hasHome = builtins.elem "interface-name:work-h0" cfg.networking.networkmanager.unmanaged;
      hasParent = builtins.elem "interface-name:eno1" cfg.networking.networkmanager.unmanaged;
    };
    expected = { hasHome = true; hasParent = false; };
  };
  "net-vm-network/home-lan-host-json-contract" = {
    expr = {
      parent = workHostEnv.externalNetwork.attachment.parentInterface;
      hostIf = workHostEnv.externalNetwork.attachment.hostIfName;
      guestIf = workHostEnv.externalNetwork.attachment.guestIfName;
      egress = workHostEnv.externalNetwork.egress.enabled;
      egressCidrs = workHostEnv.externalNetwork.egress.allowedCidrs;
      forward = builtins.head workHostEnv.externalNetwork.portForwards;
    };
    expected = {
      parent = "eno1";
      hostIf = "work-h0";
      guestIf = "external0";
      egress = true;
      egressCidrs = [ "192.168.1.0/24" ];
      forward = {
        protocol = "tcp";
        listenPort = 2222;
        sourceCidrs = [ ];
        vm = "corp-vm";
        targetIp = "10.20.0.10";
        targetPort = 22;
      };
    };
  };
  "net-vm-network/home-lan-process-contract-carries-macvtap" = {
    expr = workNetHomeProcessIface;
    expected = {
      type = "macvtap";
      id = "work-h0";
      mac = workHomeMac;
      macvtap = {
        link = "eno1";
        mode = "bridge";
      };
    };
  };
  "net-vm-network/home-lan-ch-argv-uses-macvtap-fd" = {
    expr = {
      hasFd = workNetHomeArgvFd;
      hasPlainTap = lib.any
        (arg: lib.hasInfix "mac=${workHomeMac}" arg && lib.hasInfix "tap=work-h0" arg)
        workNetChNode.argv;
    };
    expected = {
      hasFd = true;
      hasPlainTap = false;
    };
  };
  "net-vm-network/safe-home-lan-default-off" = {
    expr = {
      optionDefault = cfg.d2b.envs.safe.externalNetwork.enable;
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
      optionDefault = cfg.d2b.envs.obs.externalNetwork.enable;
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
  "net-vm-network/work-nft-home-egress-accept-present" = {
    expr = hasRule workRuleset lanToHomeAcceptRule;
    expected = true;
  };
  "net-vm-network/work-nft-home-egress-eth0-drop-guard-present" = {
    expr = hasRule workRuleset externalNetworkEth0DropRule;
    expected = true;
  };
  "net-vm-network/work-nft-home-egress-eth0-drop-before-internet" = {
    expr = beforeRule workRuleset externalNetworkEth0DropRule lanToUplinkAcceptRule;
    expected = true;
  };
  "net-vm-network/work-nft-home-egress-masquerade-present" = {
    expr = hasRule workRuleset homeMasqueradeRule;
    expected = true;
  };
  "net-vm-network/work-nft-home-port-forward-dnat-present" = {
    expr = hasRule workRuleset homeDnatRule;
    expected = true;
  };
  "net-vm-network/work-nft-home-port-forward-filter-present" = {
    expr = hasRule workRuleset homeForwardRule;
    expected = true;
  };
  "net-vm-network/work-nft-home-dhcp-client-only" = {
    expr = hasRule workRuleset ''iifname "external0" udp sport 67 udp dport 68 accept'';
    expected = true;
  };
  "net-vm-network/host-avahi-not-enabled" = {
    expr = cfg.services.avahi.enable or false;
    expected = false;
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

  # ---- external network mDNS is net-VM scoped and opt-in -----------------------
  "net-vm-network/host-avahi-disabled" = {
    expr = cfg.services.avahi.enable;
    expected = false;
  };
  "net-vm-network/workload-avahi-disabled" = {
    expr = workGuest.services.avahi.enable;
    expected = false;
  };
  "net-vm-network/work-net-avahi-enabled" = {
    expr = workNet.services.avahi.enable;
    expected = true;
  };
  "net-vm-network/work-net-avahi-reflector-enabled" = {
    expr = workNet.services.avahi.reflector;
    expected = true;
  };
  "net-vm-network/quiet-net-avahi-reflector-disabled" = {
    expr = {
      enable = quietNet.services.avahi.enable;
      reflector = quietNet.services.avahi.reflector;
    };
    expected = {
      enable = true;
      reflector = false;
    };
  };
  "net-vm-network/work-net-avahi-open-firewall-disabled" = {
    expr = workNet.services.avahi.openFirewall;
    expected = false;
  };
  "net-vm-network/work-net-avahi-interfaces" = {
    expr = workNet.services.avahi.allowInterfaces;
    expected = [ "external0" "eth1" ];
  };
  "net-vm-network/safe-net-avahi-disabled" = {
    expr = safeNet.services.avahi.enable;
    expected = false;
  };
  "net-vm-network/work-net-mdns-external0-input-rule" = {
    expr = hasRule workRuleset mdnsHomeInputRule;
    expected = true;
  };
  "net-vm-network/work-net-mdns-eth1-input-rule" = {
    expr = hasRule workRuleset mdnsLanInputRule;
    expected = true;
  };
  "net-vm-network/safe-net-mdns-external0-input-rule-absent" = {
    expr = hasRule safeRuleset mdnsHomeInputRule;
    expected = false;
  };
  "net-vm-network/safe-net-mdns-eth1-input-rule-absent" = {
    expr = hasRule safeRuleset mdnsLanInputRule;
    expected = false;
  };
  "net-vm-network/work-net-dnsmasq-local-forward" = {
    expr = builtins.elem "/local/127.0.0.1#53530" workDnsmasqServers;
    expected = true;
  };
  "net-vm-network/work-net-mdns-local-resolver-service" = {
    expr = builtins.hasAttr "d2b-mdns-local-resolver" workNet.systemd.services;
    expected = true;
  };
  "net-vm-network/safe-net-mdns-local-resolver-service-absent" = {
    expr = builtins.hasAttr "d2b-mdns-local-resolver" safeNet.systemd.services;
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
