{ lib, flakeRoot, mkEval, ... }:

let
  identity = import (flakeRoot + "/nixos-modules/v2-identity.nix");
  d2bLib = import (flakeRoot + "/nixos-modules/lib.nix") { inherit lib; };
  realmId = identity.deriveRealmId "home.local-root";
  workloadId = identity.deriveWorkloadId realmId "app";
  rawD2b = {
    site.allowUnsafeEastWest = false;
    hostLanCidrs = [ "192.168.0.0/16" ];
    realms.home = {
      enable = true;
      id = "home";
      path = "home";
      placement = "host-local";
      broker = {
        enable = true;
        hostMutation = true;
      };
      network = {
        mode = "declared";
        lanSubnet = "10.20.0.0/24";
        uplinkSubnet = "192.0.2.0/30";
        mtu = 1280;
        mssClamp = true;
        netVmName = null;
        lan.allowEastWest = false;
        hostBlocklist = [ "10.0.0.0/8" ];
        externalNetwork = {
          enable = true;
          attachment = {
            enable = true;
            interface = "eno1";
            mode = "macvtap";
            macvtapMode = "bridge";
            macAddress = null;
            ipv4 = {
              method = "dhcp";
              address = null;
              gateway = null;
              dns = [ ];
            };
          };
          egress = {
            enable = true;
            allowedCidrs = [ "192.168.1.0/24" ];
            masquerade = true;
          };
          portForwards = [{
            protocol = "tcp";
            listenPort = 2222;
            workload = "app";
            targetIp = null;
            targetPort = 22;
            sourceCidrs = [ ];
          }];
          mdns = {
            enable = true;
            reflector.enable = true;
            dnsmasqLocal = {
              enable = true;
              port = 53530;
            };
            publishWorkstation = false;
          };
        };
      };
      providers.vm = {
        enable = true;
        type = "runtime";
        implementationId = "cloud-hypervisor";
      };
      workloads.app = {
        enable = true;
        provider = "vm";
      };
    };
  };
  realmIndex =
    import (flakeRoot + "/nixos-modules/index-realms.nix") {
      inherit identity lib;
    } rawD2b.realms;
  workloadIndex =
    import (flakeRoot + "/nixos-modules/index-workloads.nix") {
      inherit identity lib;
    } {
      realms = rawD2b.realms;
      inherit realmIndex;
    };
  resourceIndex =
    import (flakeRoot + "/nixos-modules/index-resources.nix") {
      inherit identity lib;
    } {
      realms = rawD2b.realms;
      inherit realmIndex workloadIndex;
    };
  normalizedWorkloads = map
    (row: row // {
      providerBindings =
        resourceIndex.providers.bindingsByWorkloadId.${row.workloadId};
    })
    workloadIndex.enabledList;
  config.d2b = rawD2b // {
    _index = {
      realms = realmIndex;
      workloads.enabledByRealmId =
        lib.groupBy (row: row.realmId) normalizedWorkloads;
    };
  };

  plan = import (flakeRoot + "/nixos-modules/realm-network-rows.nix") {
    inherit config lib;
  };
  realm = builtins.head plan.realms;
  resources = realm.resources;
  workload = builtins.head realm.addressing.workloadRows;
  workloadTap = builtins.head resources.taps.workloads;
  forward = builtins.head realm.guest.externalNetwork.portForwards;
  providerFragment =
    import
      (flakeRoot
        + "/nixos-modules/provider-registry-v2-extensions/network.nix")
      { inherit config lib; };
  provider = builtins.head providerFragment.providers;
  invalidHostCidrPlan =
    import (flakeRoot + "/nixos-modules/realm-network-rows.nix") {
      inherit lib;
      config.d2b = config.d2b // {
        hostLanCidrs = [ "999.168.0.0/16" ];
      };
    };
  # Behavior-level allowEastWest=true fixture: same realm, but with
  # `network.lan.allowEastWest = true` and the required
  # `site.allowUnsafeEastWest = true` acknowledgement. Only the tap
  # `isolated` bridge-port flag is expected to flip; the forward/input
  # nftables rows below stay keyed on `inputRole = "uplink"` regardless,
  # since east-west enforcement never lived in nftables.
  eastWestConfig.d2b = config.d2b // {
    site = config.d2b.site // { allowUnsafeEastWest = true; };
    realms = config.d2b.realms // {
      home = config.d2b.realms.home // {
        network = config.d2b.realms.home.network // {
          lan = config.d2b.realms.home.network.lan // { allowEastWest = true; };
        };
      };
    };
  };
  eastWestPlan =
    import (flakeRoot + "/nixos-modules/realm-network-rows.nix") {
      config = eastWestConfig;
      inherit lib;
    };
  eastWestRealm = builtins.head eastWestPlan.realms;
  eastWestWorkloadTap = builtins.head eastWestRealm.resources.taps.workloads;
  allRulesMarked =
    lib.all
      (rule: rule.comment == realm.ownershipMarker)
      resources.nftables.rules;
  allChainsMarked =
    lib.all
      (chain: chain.comment == realm.ownershipMarker)
      resources.nftables.chains;
  findIndex = predicate: values:
    let
      matches = lib.filter (index: predicate (builtins.elemAt values index))
        (builtins.genList (index: index) (builtins.length values));
    in
    if matches == [ ] then null else builtins.head matches;
  hostDropIndex =
    findIndex
      (rule: lib.hasInfix "-host-drop-" rule.id)
      resources.nftables.rules;
  internetIndex =
    findIndex
      (rule: lib.hasSuffix "-internet" rule.id)
      resources.nftables.rules;
  resourceKinds = map (request: request.kind) plan.allocatorRequests;

  # Exact chain/rule-role fixtures for the host-facing forward/input/
  # postrouting rows: every one of them keys on `inputRole = "uplink"`
  # because the net VM — not the host — terminates the workload LAN, and
  # the host only ever forwards what the net VM re-emits on its uplink.
  forwardChainName = "${realm.ownershipId}-forward";
  inputChainName = "${realm.ownershipId}-input";
  postroutingChainName = "${realm.ownershipId}-postrouting";
  hostDropForwardRules = lib.filter
    (rule: lib.hasInfix "-host-drop-forward-" rule.id)
    resources.nftables.rules;
  hostDropInputRules = lib.filter
    (rule: lib.hasInfix "-host-drop-input-" rule.id)
    resources.nftables.rules;
  internetRule = lib.findFirst
    (rule: lib.hasSuffix "-internet" rule.id)
    (throw "net-vm-network: no internet forward rule in fixture")
    resources.nftables.rules;
  masqueradeRule = lib.findFirst
    (rule: lib.hasSuffix "-masquerade" rule.id)
    (throw "net-vm-network: no masquerade postrouting rule in fixture")
    resources.nftables.rules;
  deadBridgeRules = lib.filter
    (rule: lib.hasInfix "-lan-netvm" rule.id || lib.hasInfix "-peer-default" rule.id)
    resources.nftables.rules;
  expectedGeneratedExternalMac = d2bLib.mkMac realmId "external" 0;

  # Real end-to-end reachability proof, on top of the hand-rolled `plan`
  # fixture above (which only proves realm-network-rows.nix's own pure
  # output shape). This builds an ordinary consumer-shaped realm through
  # the actual module system: a realm with `network.mode = "declared"`
  # and one regular declared workload ("corp-vm"). It then asserts the
  # auto-declared "network" workload (a) is actually reachable from that
  # realm's own `d2b.realms.<realm>` config (not merely a pure-function
  # row), (b) flows through the ordinary workload/role/resource index into
  # the composed processes.json / minijail-profiles / `_computedWorkloads`
  # bundle artifacts alongside its declared sibling, and (c) that sibling's
  # own composed DAG/argv are unaffected by the net VM's presence.
  workRealmId = identity.deriveRealmId "work.local-root";
  netVmWorkloadId = identity.deriveWorkloadId workRealmId "network";
  corpVmWorkloadId = identity.deriveWorkloadId workRealmId "corp-vm";
  netVmCloudHypervisorRoleId =
    identity.deriveRoleId workRealmId netVmWorkloadId "cloud-hypervisor";
  reachabilityManifest =
    builtins.fromJSON reachability.d2b._manifestPkg.text;

  reachability = (mkEval [
    {
      boot.loader.grub.enable = false;
      boot.loader.systemd-boot.enable = false;
      boot.initrd.includeDefaultModules = false;
      fileSystems."/" = {
        device = "tmpfs";
        fsType = "tmpfs";
      };
      environment.etc."machine-id".text =
        "00000000000000000000000000000000";
      system.stateVersion = "25.11";
      users.users.alice = {
        isNormalUser = true;
        uid = 1000;
      };
      d2b.acceptDestructiveV2Cutover = true;
      d2b.site = {
        waylandUser = "alice";
        launcherUsers = [ "alice" ];
        yubikey.enable = false;
      };
      d2b.realms.work = {
        path = "work";
        placement = "host-local";
        broker = {
          enable = true;
          hostMutation = true;
        };
        network = {
          mode = "declared";
          lanSubnet = "10.90.0.0/24";
          uplinkSubnet = "192.0.2.40/30";
        };
        providers.runtime = {
          type = "runtime";
          implementationId = "cloud-hypervisor";
        };
        workloads.corp-vm = {
          providerRefs.runtime = "runtime";
          config = {
            networking.hostName = lib.mkDefault "corp-vm";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };
      };
    }
  ]).config;

  reachabilityNetwork = lib.findFirst
    (realm: realm.canonicalRealmId == workRealmId)
    (throw "reachability: realm work.local-root has no network rows")
    reachability.d2b._realmNetwork.realms;
  reachabilityNetGuest = reachabilityNetwork.guest;
  corpVmNetwork = lib.findFirst
    (row: row.canonicalWorkloadId == corpVmWorkloadId)
    (throw "reachability: corp-vm has no network row")
    reachabilityNetwork.addressing.workloadRows;

  netVmDag = builtins.head (builtins.filter
    (row: row.workloadIdentity.canonicalTarget == "network.work.local-root.d2b")
    reachability.d2b._bundle.processesJson.data.vms);
  corpVmDag = builtins.head (builtins.filter
    (row: row.workloadIdentity.canonicalTarget == "corp-vm.work.local-root.d2b")
    reachability.d2b._bundle.processesJson.data.vms);
  netVmRolesByKind = lib.sort lib.lessThan
    (map (node: node.role) netVmDag.nodes);
  corpVmRolesByKind = lib.sort lib.lessThan
    (map (node: node.role) corpVmDag.nodes);
  netVmHypervisor = lib.findFirst
    (node: node.role == "cloud-hypervisor-runner")
    (throw "missing net VM cloud-hypervisor-runner node")
    netVmDag.nodes;
  corpVmHypervisor = lib.findFirst
    (node: node.role == "cloud-hypervisor-runner")
    (throw "missing corp-vm cloud-hypervisor-runner node")
    corpVmDag.nodes;
  netFlagIndex = argv:
    findIndex (arg: arg == "--net") argv;
  netVmNetArgs =
    let idx = netFlagIndex netVmHypervisor.argv;
    in lib.sublist (idx + 1) 2 netVmHypervisor.argv;
  corpVmNetArgs =
    let idx = netFlagIndex corpVmHypervisor.argv;
    in lib.sublist (idx + 1) 1 corpVmHypervisor.argv;
in
{
  "net-vm-network/realm-plan-assertions-pass" = {
    expr = builtins.all (assertion: assertion.assertion) plan.assertions;
    expected = true;
  };

  "net-vm-network/consumes-normalized-identities" = {
    expr = {
      realm = realm.canonicalRealmId;
      workload = workload.canonicalWorkloadId;
    };
    expected = {
      realm = realmId;
      workload = workloadId;
    };
  };

  "net-vm-network/invalid-host-cidr-fails-closed" = {
    expr =
      builtins.any (assertion: !assertion.assertion)
        invalidHostCidrPlan.assertions;
    expected = true;
  };

  "net-vm-network/allocator-resource-kinds" = {
    expr = resourceKinds;
    expected = [
      "bridge"
      "bridge"
      "veth-pair"
      "tap"
      "tap"
      "tap"
      "nftables-partition"
    ];
  };

  "net-vm-network/canonical-resource-ids" = {
    expr = lib.all
      (request:
        builtins.match "^[a-z][a-z0-9-]*$" request.resourceId != null)
      plan.allocatorRequests;
    expected = true;
  };

  "net-vm-network/interface-names-bounded" = {
    expr = lib.all
      (name: builtins.stringLength name <= 15)
      realm.coexistence.networkManager.unmanagedIfNames;
    expected = true;
  };

  "net-vm-network/workload-address" = {
    expr = workload.ip;
    expected = "10.20.0.10";
  };

  "net-vm-network/port-forward-target" = {
    expr = forward.targetIp;
    expected = "10.20.0.10";
  };

  "net-vm-network/mtu-propagates-to-bridges" = {
    expr = {
      lan = resources.bridges.lan.mtu;
      uplink = resources.bridges.uplink.mtu;
    };
    expected = {
      lan = 1280;
      uplink = 1280;
    };
  };

  "net-vm-network/mtu-propagates-to-taps" = {
    expr = {
      netLan = resources.taps.netVm.lan.mtu;
      workload = workloadTap.mtu;
    };
    expected = {
      netLan = 1280;
      workload = 1280;
    };
  };

  "net-vm-network/link-attachment-references" = {
    expr = {
      vethNamespace = resources.veth.namespaceResourceId;
      vethBridge = resources.veth.realmBridgeResourceId;
      hostAddress = resources.veth.hostAddress;
      hostRoute = builtins.head resources.veth.routes;
      netUplinkBridge = resources.taps.netVm.uplink.bridgeResourceId;
      netLanBridge = resources.taps.netVm.lan.bridgeResourceId;
      workloadBridge = workloadTap.bridgeResourceId;
    };
    expected = {
      vethNamespace = realm.namespaceResourceId;
      vethBridge = resources.bridges.uplink.resourceId;
      hostAddress = {
        address = "192.0.2.1";
        prefixLength = "30";
      };
      hostRoute = {
        destination = "10.20.0.0/24";
        via = "192.0.2.2";
      };
      netUplinkBridge = resources.bridges.uplink.resourceId;
      netLanBridge = resources.bridges.lan.resourceId;
      workloadBridge = resources.bridges.lan.resourceId;
    };
  };

  "net-vm-network/mss-clamp" = {
    expr = {
      enabled = realm.policy.mssClamp;
      bytes = realm.policy.mssClampBytes;
      rulePresent =
        lib.any
          (rule: rule.action == "clamp-mss-to-pmtu")
          resources.nftables.rules;
    };
    expected = {
      enabled = true;
      bytes = 1240;
      rulePresent = true;
    };
  };

  "net-vm-network/default-workload-isolation" = {
    expr = {
      policy = realm.policy.workloadTapIsolation;
      tap = workloadTap.isolated;
      netVm = resources.taps.netVm.lan.isolated;
    };
    expected = {
      policy = true;
      tap = true;
      netVm = false;
    };
  };

  "net-vm-network/allow-east-west-true-unisolates-workload-tap" = {
    expr = {
      policy = eastWestRealm.policy.workloadTapIsolation;
      tap = eastWestWorkloadTap.isolated;
      netVm = eastWestRealm.resources.taps.netVm.lan.isolated;
    };
    expected = {
      policy = false;
      tap = false;
      netVm = false;
    };
  };

  "net-vm-network/no-dead-bridge-forward-rules" = {
    expr = deadBridgeRules;
    expected = [ ];
  };

  "net-vm-network/host-drops-cover-forward-and-input-chains" = {
    expr = {
      forwardCount = builtins.length hostDropForwardRules;
      inputCount = builtins.length hostDropInputRules;
      forwardChains = lib.unique (map (rule: rule.chain) hostDropForwardRules);
      inputChains = lib.unique (map (rule: rule.chain) hostDropInputRules);
      forwardInputRoles =
        lib.unique (map (rule: rule.match.inputRole) hostDropForwardRules);
      inputInputRoles =
        lib.unique (map (rule: rule.match.inputRole) hostDropInputRules);
    };
    expected = {
      forwardCount = builtins.length realm.policy.hostBlocklist;
      inputCount = builtins.length realm.policy.hostBlocklist;
      forwardChains = [ forwardChainName ];
      inputChains = [ inputChainName ];
      forwardInputRoles = [ "uplink" ];
      inputInputRoles = [ "uplink" ];
    };
  };

  "net-vm-network/internet-forward-matches-uplink-input-role" = {
    expr = {
      chain = internetRule.chain;
      action = internetRule.action;
      match = internetRule.match;
    };
    expected = {
      chain = forwardChainName;
      action = "accept";
      match = { inputRole = "uplink"; };
    };
  };

  "net-vm-network/masquerade-matches-uplink-input-role" = {
    expr = {
      chain = masqueradeRule.chain;
      action = masqueradeRule.action;
      match = masqueradeRule.match;
    };
    expected = {
      chain = postroutingChainName;
      action = "masquerade";
      match = { inputRole = "uplink"; };
    };
  };

  "net-vm-network/generated-external-mac-used-for-guest" = {
    expr = {
      guestMac = realm.guest.externalNetwork.attachment.guestMac;
      hostMac = realm.guest.externalNetwork.attachment.macAddress;
    };
    expected = {
      guestMac = expectedGeneratedExternalMac;
      hostMac = expectedGeneratedExternalMac;
    };
  };

  "net-vm-network/nft-rows-carry-ownership-marker" = {
    expr = allRulesMarked && allChainsMarked;
    expected = true;
  };

  "net-vm-network/nft-foreign-state-fails-closed" = {
    expr = resources.nftables.foreignRulePolicy;
    expected = "preserve-and-fail-closed";
  };

  "net-vm-network/host-drops-precede-internet-accept" = {
    expr =
      hostDropIndex != null
      && internetIndex != null
      && hostDropIndex < internetIndex;
    expected = true;
  };

  "net-vm-network/network-manager-marked-block" = {
    expr = realm.coexistence.networkManager;
    expected = realm.coexistence.networkManager // {
      configPath = "/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf";
      beginMarker = "# d2b-managed begin";
      endMarker = "# d2b-managed end";
      foreignConflict = "nm-managed-foreign-conflict";
    };
  };

  "net-vm-network/networkd-detect-only" = {
    expr = realm.coexistence.networkd;
    expected = {
      mode = "detect-only";
      requiredUnmanagedPrefix = "d2b-";
      foreignConflict = "networkd-managed-foreign-conflict";
    };
  };

  "net-vm-network/child-broker-lease-authority" = {
    expr = realm.namespaceLocalEffects.authority;
    expected = "allocator-lease-fds-only";
  };

  "net-vm-network/provider-fragment-axis" = {
    expr = {
      axis = providerFragment.axis;
      implementation = provider.descriptor.implementationId;
      binding = provider.binding.axis;
      capabilities = provider.descriptor.capabilities;
    };
    expected = {
      axis = "network";
      implementation = "local-realm";
      binding = "network";
      capabilities = [
        "network.plan"
        "network.ensure"
        "network.inspect"
        "network.adopt"
        "network.release"
      ];
    };
  };

  "net-vm-network/auto-declared-from-realm-network-mode" = {
    expr = {
      workload = {
        inherit (reachability.d2b.realms.work.workloads.network)
          providerRefs autostart;
      };
      provider = {
        inherit (reachability.d2b.realms.work.providers.network-vm-runtime)
          type implementationId;
      };
    };
    expected = {
      workload = {
        providerRefs.runtime = "network-vm-runtime";
        autostart = true;
      };
      provider = {
        type = "runtime";
        implementationId = "cloud-hypervisor";
      };
    };
  };

  "net-vm-network/reachable-through-generic-workload-index" = {
    expr = {
      indexed = reachability.d2b._index.workloads.byId ? ${netVmWorkloadId};
      runtimeImplementation =
        reachability.d2b._index.workloads.byId.${netVmWorkloadId}
          .providerBindings.runtime.implementationId;
      computed = reachability.d2b._computedWorkloads ? ${netVmWorkloadId};
    };
    expected = {
      indexed = true;
      runtimeImplementation = "cloud-hypervisor";
      computed = true;
    };
  };

  "net-vm-network/composed-dag-has-expected-role-set" = {
    expr = lib.unique netVmRolesByKind;
    expected = lib.sort lib.lessThan [
      "cloud-hypervisor-runner"
      "guest-control-health"
      "store-virtiofs-preflight"
      "vsock-relay"
      "virtiofsd"
    ];
  };

  "net-vm-network/composed-dag-net-argv-matches-guest-metadata" = {
    expr = netVmNetArgs;
    expected = [
      "tap=${reachabilityNetGuest.interfaces.netVmUplink},mac=${reachabilityNetGuest.netUplinkMac}"
      "tap=${reachabilityNetGuest.interfaces.netVmLan},mac=${reachabilityNetGuest.netLanMac}"
    ];
  };

  "net-vm-network/eth-dhcp-neutralizer-survives-real-composition" = {
    expr =
      let
        netCfg = reachability.d2b._computedWorkloads.${netVmWorkloadId}
          .config.systemd.network.networks."10-eth-dhcp";
      in {
        mac = netCfg.matchConfig.MACAddress;
        enable = netCfg.enable;
      };
    expected = {
      mac = "00:00:00:00:00:00";
      enable = false;
    };
  };

  "net-vm-network/public-manifest-identifies-network-workload" = {
    expr = {
      netVm = {
        inherit (reachabilityManifest.${netVmWorkloadId})
          bridge
          isNetVm
          netVm
          staticIp
          tap
          ;
      };
      workload = {
        inherit (reachabilityManifest.${corpVmWorkloadId})
          bridge
          isNetVm
          netVm
          staticIp
          tap
          usbipdHostIp
          ;
      };
    };
    expected = {
      netVm = {
        bridge = reachabilityNetwork.resources.bridges.uplink.ifName;
        isNetVm = true;
        netVm = null;
        staticIp = reachabilityNetwork.addressing.uplink.netVm;
        tap = reachabilityNetwork.resources.taps.netVm.uplink.ifName;
      };
      workload = {
        bridge = reachabilityNetwork.resources.bridges.lan.ifName;
        isNetVm = false;
        netVm = netVmWorkloadId;
        staticIp = corpVmNetwork.ip;
        tap = corpVmNetwork.tap.ifName;
        usbipdHostIp = reachabilityNetwork.addressing.uplink.host;
      };
    };
  };

  "net-vm-network/minijail-profile-present-for-net-vm-runtime-role" = {
    expr =
      let
        profile = reachability.d2b._bundle.minijailProfiles
          ."role-${netVmCloudHypervisorRoleId}".data;
      in {
        profileId = profile.profileId;
        cgroupSubtree = profile.cgroupPlacement.subtree;
      };
    expected = {
      profileId = "role-${netVmCloudHypervisorRoleId}";
      cgroupSubtree =
        "d2b.slice/r-${workRealmId}/workloads/w-${netVmWorkloadId}/${netVmCloudHypervisorRoleId}";
    };
  };

  "net-vm-network/non-network-workload-unaffected" = {
    expr = {
      roles = lib.unique corpVmRolesByKind;
      netArgsCount = builtins.length corpVmNetArgs;
      workloadCount =
        builtins.length reachability.d2b._bundle.processesJson.data.vms;
    };
    expected = {
      roles = lib.sort lib.lessThan [
        "cloud-hypervisor-runner"
        "guest-control-health"
        "store-virtiofs-preflight"
        "vsock-relay"
        "virtiofsd"
      ];
      netArgsCount = 1;
      # Exactly the auto-declared net VM plus the one consumer-declared
      # workload — no extra workloads leak in from the auto-declaration.
      workloadCount = 2;
    };
  };
}
