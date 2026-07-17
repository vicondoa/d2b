{ lib, flakeRoot, ... }:

let
  identity = import (flakeRoot + "/nixos-modules/v2-identity.nix");
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
  netSource = builtins.readFile (flakeRoot + "/nixos-modules/net.nix");
  hostSource = builtins.readFile (flakeRoot + "/nixos-modules/network.nix");
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

  "net-vm-network/eth-dhcp-match-mac-sentinel" = {
    expr =
      lib.hasInfix ''"10-eth-dhcp" = lib.mkForce'' netSource
      && lib.hasInfix ''matchConfig.MACAddress = "00:00:00:00:00:00";''
        netSource;
    expected = true;
  };

  "net-vm-network/no-env-host-materialization" = {
    expr =
      !lib.hasInfix "cfg.envs" hostSource
      && !lib.hasInfix "d2b.vms" hostSource
      && !lib.hasInfix "systemd.network" hostSource
      && !lib.hasInfix "networking.nat" hostSource;
    expected = true;
  };
}
