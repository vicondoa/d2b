{ config, lib }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  identity = import ./v2-identity.nix;

  sortNames = lib.sort lib.lessThan;

  enabledRealmRows = cfg._index.realms.enabledList;
  declaredRealmRows = lib.filter
    (row: cfg.realms.${row.realmName}.network.mode == "declared")
    enabledRealmRows;

  ifName = tag: seed:
    "d2b-${tag}${lib.toUpper
      (builtins.substring 0 8 (builtins.hashString "sha256" seed))}";

  allocatorRequest =
    { realmPath
    , resourceId
    , kind
    , share ? "exclusive"
    , phase
    , ordinal
    , sourceKind
    , refName
    }:
    {
      inherit realmPath resourceId kind share;
      acquisitionOrder = { inherit phase ordinal; };
      source = {
        kind = sourceKind;
        refName = refName;
      };
    };

  cidrParts = cidr:
    if cidr == null then [ ] else lib.splitString "/" cidr;
  cidrMask = cidr:
    let parts = cidrParts cidr;
    in if builtins.length parts == 2 then builtins.elemAt parts 1 else null;
  cidrBase = cidr:
    let parts = cidrParts cidr;
    in if parts == [ ] then null else builtins.head parts;
  ipv4Octet = "(25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])";
  validIpv4Cidr = cidr:
    cidr != null
    && builtins.match
      "${ipv4Octet}[.]${ipv4Octet}[.]${ipv4Octet}[.]${ipv4Octet}/([0-9]|[12][0-9]|3[0-2])"
      cidr != null;
  subnetIp = cidr: index:
    if !validIpv4Cidr cidr then null else d2bLib.subnetIp cidr index;

  # "network" is the reserved workload name for the auto-declared net VM
  # (see options-realms-workloads.nix, which self-declares
  # `workloads.network` + `providers.network-vm-runtime` for every realm
  # with `network.mode == "declared"`). It must never be treated as one of
  # the realm's own LAN-participant workloads: it has no per-workload LAN
  # tap/DHCP reservation of its own (it terminates the uplink+LAN taps
  # computed below instead), and workload-process-rows.nix gives it the
  # realm's netVm interfaces directly.
  realmWorkloads = realmRow:
    lib.filter
      (workload:
        let
          runtime =
            lib.attrByPath
              [ "providerBindings" "runtime" "implementationId" ]
              null
              workload;
        in
        workload.enabled
        && workload.workloadName != "network"
        && builtins.elem runtime [ "cloud-hypervisor" "qemu-media" ])
      (cfg._index.workloads.enabledByRealmId.${realmRow.realmId} or [ ]);

  # `isolated` tracks the realm's explicit east-west policy: workload TAPs
  # are bridge-port-isolated by default, and only lose isolation when
  # `network.lan.allowEastWest = true` (which itself requires
  # `d2b.site.allowUnsafeEastWest = true`, asserted below). This is the
  # actual east-west enforcement point — same-LAN-bridge traffic between
  # workload taps is L2-local and never reaches the IP forward hook, so
  # there is no corresponding nftables forward rule to gate it.
  workloadRow = canonicalRealmId: lanSubnet: isolated: ordinal: workload:
    let
      index = ordinal + 10;
    in
    {
      inherit (workload) workloadName canonicalTarget;
      canonicalWorkloadId = workload.workloadId;
      inherit index;
      ip = subnetIp lanSubnet index;
      mac = d2bLib.mkMac canonicalRealmId "lan" index;
      tap = {
        resourceId = "tap-${workload.workloadId}";
        ifName = ifName "t" "workload:${canonicalRealmId}:${workload.workloadId}";
        bridgeRole = "workload-lan";
        inherit isolated;
      };
    };

  normalizedPortForward = workloadByName: pf:
    let
      workload =
        if pf.workload != null && builtins.hasAttr pf.workload workloadByName
        then workloadByName.${pf.workload}
        else null;
    in
    {
      inherit (pf) protocol listenPort sourceCidrs;
      workloadId =
        if workload == null then null else workload.canonicalWorkloadId;
      targetIp =
        if pf.targetIp != null then pf.targetIp
        else if workload == null then null
        else workload.ip;
      targetPort =
        if pf.targetPort == null then pf.listenPort else pf.targetPort;
    };

  realmRow = realmIndexRow:
    let
      realmName = realmIndexRow.realmName;
      realm = cfg.realms.${realmName};
      canonicalRealmId = realmIndexRow.realmId;
      networkId = "net-${canonicalRealmId}";
      namespaceResourceId = "realm-${canonicalRealmId}-netns";
      ownershipId = "r-${canonicalRealmId}";
      ownershipMarker = "d2b managed: ${ownershipId}";
      network = realm.network;
      workloadRows = lib.imap0
        (workloadRow canonicalRealmId network.lanSubnet
          (!network.lan.allowEastWest))
        (realmWorkloads realmIndexRow);
      workloadByName = lib.listToAttrs (map
        (workload: {
          name = workload.workloadName;
          value = workload;
        })
        workloadRows);
      lanBridge = {
        resourceId = "${networkId}-lan";
        ifName = ifName "b" "lan:${canonicalRealmId}";
        role = "workload-lan";
      };
      uplinkBridge = {
        resourceId = "${networkId}-uplink";
        ifName = ifName "b" "uplink:${canonicalRealmId}";
        role = "uplink";
      };
      namespaceVeth = {
        resourceId = "${networkId}-veth";
        hostIfName = ifName "v" "veth-host:${canonicalRealmId}";
        realmIfName = ifName "v" "veth-realm:${canonicalRealmId}";
      };
      netVmUplinkTap = {
        resourceId = "${networkId}-net-up";
        ifName = ifName "t" "net-uplink:${canonicalRealmId}";
        bridgeRole = "uplink";
        isolated = true;
      };
      netVmLanTap = {
        resourceId = "${networkId}-net-lan";
        ifName = ifName "t" "net-lan:${canonicalRealmId}";
        bridgeRole = "net-vm-lan";
        isolated = false;
      };
      chainName = hook: "${ownershipId}-${hook}";
      markedRule = id: hook: action: match: {
        inherit id action match;
        chain = chainName hook;
        comment = ownershipMarker;
      };
      markedChain = chain: chain // {
        name = chainName chain.name;
        comment = ownershipMarker;
      };
      # Host-blocked destinations, shared by the forward/input host-drop
      # rows below and by `policy.hostBlocklist` / `guest.hostBlocklist`.
      effectiveHostBlocklist =
        sortNames (lib.unique (network.hostBlocklist ++ cfg.hostLanCidrs));
      hostDropRule = chainHook: ordinal: cidr:
        markedRule "${networkId}-host-drop-${chainHook}-${toString ordinal}"
          chainHook "drop" {
            inputRole = "uplink";
            destinationCidr = cidr;
          };
      # Host-side traffic never sees the workload LAN bridge directly:
      # the net VM terminates the LAN and re-emits everything bound for
      # the host/internet on its uplink tap. So every host-facing
      # forward/input/postrouting match below keys on `inputRole =
      # "uplink"`, not the workload-lan bridge role. East-west traffic
      # between workload taps on the same LAN bridge is L2-local and
      # never reaches the IP forward hook at all; it is gated purely by
      # the bridge port `isolated` flag (see `workloadRow`'s
      # `!network.lan.allowEastWest` tap isolation), so there is no
      # "lan-netvm" / "peer-default" forward rule to write here.
      nftRules = [
        (markedRule "${networkId}-established" "forward" "accept" {
          connectionState = [ "established" "related" ];
        })
        (markedRule "${networkId}-invalid" "forward" "drop" {
          connectionState = [ "invalid" ];
        })
      ]
      ++ lib.optional network.mssClamp
        (markedRule "${networkId}-mss-clamp" "forward" "clamp-mss-to-pmtu" {
          tcpSyn = true;
        })
      ++ lib.imap0 (hostDropRule "forward") effectiveHostBlocklist
      ++ lib.imap0 (hostDropRule "input") effectiveHostBlocklist
      ++ [
        (markedRule "${networkId}-internet" "forward" "accept" {
         inputRole = "uplink";
        })
        (markedRule "${networkId}-masquerade" "postrouting" "masquerade" {
         inputRole = "uplink";
        })
      ];
      externalIfName = ifName "x" "external:${canonicalRealmId}";
      externalMac =
        if network.externalNetwork.attachment.macAddress != null
        then network.externalNetwork.attachment.macAddress
        else d2bLib.mkMac canonicalRealmId "external" 0;
      netVmWorkloadId =
        identity.deriveWorkloadId canonicalRealmId "network";
      netVmRoleId =
        identity.deriveRoleId canonicalRealmId netVmWorkloadId
          "cloud-hypervisor";
      portForwards =
        map (normalizedPortForward workloadByName)
          network.externalNetwork.portForwards;
      managedIfNames = [
        lanBridge.ifName
        uplinkBridge.ifName
        namespaceVeth.hostIfName
        namespaceVeth.realmIfName
        netVmUplinkTap.ifName
        netVmLanTap.ifName
      ] ++ map (workload: workload.tap.ifName) workloadRows
        ++ lib.optional network.externalNetwork.attachment.enable externalIfName;
      allocatorRequests = [
        (allocatorRequest {
          realmPath = realmIndexRow.realmPath;
          resourceId = lanBridge.resourceId;
          kind = "bridge";
          share = "shared-partition";
          phase = 32;
          ordinal = 0;
          sourceKind = "realm-network";
          refName = networkId;
        })
        (allocatorRequest {
          realmPath = realmIndexRow.realmPath;
          resourceId = uplinkBridge.resourceId;
          kind = "bridge";
          share = "shared-partition";
          phase = 32;
          ordinal = 1;
          sourceKind = "realm-network";
          refName = networkId;
        })
        (allocatorRequest {
          realmPath = realmIndexRow.realmPath;
          resourceId = namespaceVeth.resourceId;
          kind = "veth-pair";
          phase = 33;
          ordinal = 0;
          sourceKind = "realm-network";
          refName = networkId;
        })
        (allocatorRequest {
          realmPath = realmIndexRow.realmPath;
          resourceId = netVmUplinkTap.resourceId;
          kind = "tap";
          phase = 34;
          ordinal = 0;
          sourceKind = "realm-network";
          refName = networkId;
        })
        (allocatorRequest {
          realmPath = realmIndexRow.realmPath;
          resourceId = netVmLanTap.resourceId;
          kind = "tap";
          phase = 34;
          ordinal = 1;
          sourceKind = "realm-network";
          refName = networkId;
        })
      ] ++ lib.imap0
        (ordinal: workload:
          allocatorRequest {
            realmPath = realmIndexRow.realmPath;
            resourceId = workload.tap.resourceId;
            kind = "tap";
            phase = 34;
            ordinal = ordinal + 2;
            sourceKind = "realm-workload-network";
            refName = workload.canonicalWorkloadId;
          })
        workloadRows
      ++ [
        (allocatorRequest {
          realmPath = realmIndexRow.realmPath;
          resourceId = "${networkId}-nft";
          kind = "nftables-partition";
          share = "shared-partition";
          phase = 35;
          ordinal = 0;
          sourceKind = "realm-network";
          refName = networkId;
        })
      ];
    in
    {
      inherit
        realmName
        canonicalRealmId
        networkId
        ownershipId
        ownershipMarker
        allocatorRequests
        netVmWorkloadId
        netVmRoleId
        ;
      configuredRealmId = realmIndexRow.metadata.configuredId;
      realmPath = realmIndexRow.realmPath;
      generation = 1;
      mode = "declared";
      inherit namespaceResourceId;
      resources = {
        bridges = {
          lan = lanBridge // {
            mtu = network.mtu;
            stp = false;
            multicastSnooping = false;
            ipv6Disabled = true;
          };
          uplink = uplinkBridge // {
            mtu = network.mtu;
            stp = false;
            multicastSnooping = false;
            ipv6Disabled = true;
          };
        };
        veth = namespaceVeth // {
          inherit namespaceResourceId;
          realmBridgeResourceId = uplinkBridge.resourceId;
          hostAddress = {
            address = subnetIp network.uplinkSubnet 1;
            prefixLength = cidrMask network.uplinkSubnet;
          };
          routes = [{
            destination = network.lanSubnet;
            via = subnetIp network.uplinkSubnet 2;
          }];
          mtu = network.mtu;
          ipv6Disabled = true;
        };
        taps = {
          netVm = {
            uplink = netVmUplinkTap // {
              bridgeResourceId = uplinkBridge.resourceId;
              mtu = network.mtu;
            };
            lan = netVmLanTap // {
              bridgeResourceId = lanBridge.resourceId;
              mtu = network.mtu;
            };
          };
          workloads = map
            (workload: workload.tap // {
              workloadId = workload.canonicalWorkloadId;
              bridgeResourceId = lanBridge.resourceId;
              mtu = network.mtu;
            })
            workloadRows;
        };
        nftables = {
          resourceId = "${networkId}-nft";
          family = "inet";
          table = "d2b";
          partitionId = ownershipId;
          marker = ownershipMarker;
          foreignRulePolicy = "preserve-and-fail-closed";
          chains = map markedChain [
            {
              name = "prerouting";
              hook = "prerouting";
              priority = -150;
              policy = "accept";
            }
            {
              name = "forward";
              hook = "forward";
              priority = -5;
              policy = "drop";
            }
            {
              name = "output";
              hook = "output";
              priority = -5;
              policy = "accept";
            }
            {
              name = "input";
              hook = "input";
              priority = -5;
              policy = "accept";
            }
            {
              name = "postrouting";
              hook = "postrouting";
              priority = 100;
              policy = "accept";
            }
          ];
          rules = nftRules;
        };
      };
      addressing = {
        lan = {
          cidr = network.lanSubnet;
          gateway = subnetIp network.lanSubnet 1;
          mask = cidrMask network.lanSubnet;
        };
        uplink = {
          cidr = network.uplinkSubnet;
          host = subnetIp network.uplinkSubnet 1;
          netVm = subnetIp network.uplinkSubnet 2;
          mask = cidrMask network.uplinkSubnet;
        };
        inherit workloadRows;
      };
      policy = {
        allowEastWest = network.lan.allowEastWest;
        workloadTapIsolation = !network.lan.allowEastWest;
        netVmTapIsolation = false;
        uplinkIsolation = true;
        hostBlocklist = effectiveHostBlocklist;
        mtu = network.mtu;
        mssClamp = network.mssClamp;
        mssClampBytes =
          if network.mtu == null then null else network.mtu - 40;
        ipv6 = {
          disable = true;
          acceptRa = false;
          autoconf = false;
        };
      };
      coexistence = {
        ownership = {
          marker = ownershipMarker;
          foreignConflict = "foreign-nft-rule-preserved";
        };
        networkManager = {
          configPath = "/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf";
          beginMarker = "# d2b-managed begin";
          endMarker = "# d2b-managed end";
          unmanagedIfNames = managedIfNames;
          foreignConflict = "nm-managed-foreign-conflict";
        };
        networkd = {
          mode = "detect-only";
          requiredUnmanagedPrefix = "d2b-";
          foreignConflict = "networkd-managed-foreign-conflict";
        };
      };
      namespaceLocalEffects = {
        owner = "realm-child-broker";
        authority = "allocator-lease-fds-only";
        operations = [
          "configure-addresses"
          "configure-dhcp"
          "configure-dns"
          "configure-nat"
          "configure-filter"
          "configure-mss-clamp"
        ];
      };
      providerBinding = {
        inherit networkId;
        allocatorLeaseId = "lease-${canonicalRealmId}-network";
        bridgeSetId = "${networkId}-bridges";
        tapSetId = "${networkId}-taps";
        inherit netVmRoleId;
        natPolicyId = "${networkId}-nat";
        dhcpPolicyId = "${networkId}-dhcp";
        nftPolicyId = "${networkId}-nft";
        netlinkPolicyId = "${networkId}-netlink";
        externalAttachmentId =
          if network.externalNetwork.attachment.enable
          then "${networkId}-external"
          else null;
        resourceGeneration = 1;
      };
      guest = {
        name = canonicalRealmId;
        netName =
          if network.netVmName == null
          then "sys-${realmIndexRow.metadata.configuredId}-net"
          else network.netVmName;
        netUplinkMac = d2bLib.mkMac canonicalRealmId "up" 2;
        netLanMac = d2bLib.mkMac canonicalRealmId "lan" 1;
        netUplinkIp = subnetIp network.uplinkSubnet 2;
        hostUplinkIp = subnetIp network.uplinkSubnet 1;
        netLanIp = subnetIp network.lanSubnet 1;
        uplinkMask = cidrMask network.uplinkSubnet;
        lanMask = cidrMask network.lanSubnet;
        lanSubnet = network.lanSubnet;
        dhcpRangeStart = subnetIp network.lanSubnet 100;
        dhcpRangeEnd = subnetIp network.lanSubnet 200;
        workloads = lib.listToAttrs (map
          (workload: {
            name = workload.workloadName;
            value = {
              inherit (workload) ip mac;
            };
          })
          workloadRows);
        inherit ownershipId;
        mtu = network.mtu;
        mssClamp = network.mssClamp;
        allowEastWest = network.lan.allowEastWest;
        hostBlocklist = effectiveHostBlocklist;
        interfaces = {
          netVmUplink = netVmUplinkTap.ifName;
          netVmLan = netVmLanTap.ifName;
        };
        externalNetwork = {
          attachment = {
            inherit (network.externalNetwork.attachment)
              enable
              interface
              mode
              macvtapMode
              ipv4
              ;
            hostIfName = externalIfName;
            guestIfName = "external0";
            # `macAddress` stays the resolved (never-null) value consumed by
            # the host-side macvtap attachment (workload-process-rows.nix).
            # `guestMac` is the same resolved value under a name that can
            # never be confused with the nullable configured
            # `network.externalNetwork.attachment.macAddress` option — the
            # net VM guest module (net.nix) matches its `10-home` link on
            # `guestMac` for exactly that reason.
            macAddress = externalMac;
            guestMac = externalMac;
          };
          inherit (network.externalNetwork) egress mdns;
          inherit portForwards;
        };
      };
    };

  realms = map realmRow declaredRealmRows;
  allocatorRequests = lib.flatten (map
    (realm: realm.allocatorRequests)
    realms);

  declaredCidrs = lib.flatten (map
    (realm:
      lib.filter validIpv4Cidr [
        realm.addressing.lan.cidr
        realm.addressing.uplink.cidr
      ])
    realms);
  duplicateResourceIds =
    lib.attrNames (lib.filterAttrs (_: rows: builtins.length rows > 1)
      (lib.groupBy (row: row.resourceId) allocatorRequests));
  allIfNames = lib.flatten (map
    (realm: realm.coexistence.networkManager.unmanagedIfNames)
    realms);
  duplicateIfNames =
    lib.attrNames (lib.filterAttrs (_: names: builtins.length names > 1)
      (lib.groupBy (name: name) allIfNames));

  perRealmAssertions = lib.concatMap
    (realmIndexRow:
      let
        realmName = realmIndexRow.realmName;
        realm = cfg.realms.${realmName};
        network = realm.network;
        networkWorkloads = realmWorkloads realmIndexRow;
        workloadNames = map (workload: workload.workloadName) networkWorkloads;
        lanOctets =
          if cidrBase network.lanSubnet == null
          then [ ]
          else lib.splitString "." (cidrBase network.lanSubnet);
      in
      [
        {
          assertion =
            realm.placement == "host-local"
            && realm.broker.enable
            && realm.broker.hostMutation;
          message = "d2b.realms.${realmName}.network.mode = \"declared\" requires a host-local realm with an enabled host-mutation broker.";
        }
        {
          assertion =
            validIpv4Cidr network.lanSubnet
            && cidrMask network.lanSubnet == "24"
            && builtins.length lanOctets == 4
            && lib.last lanOctets == "0";
          message = "d2b.realms.${realmName}.network.lanSubnet must be an IPv4 /24 network ending in .0.";
        }
        {
          assertion =
            validIpv4Cidr network.uplinkSubnet
            && cidrMask network.uplinkSubnet == "30";
          message = "d2b.realms.${realmName}.network.uplinkSubnet must be an IPv4 /30.";
        }
        {
          assertion = builtins.length networkWorkloads <= 241;
          message = "d2b.realms.${realmName}: a declared /24 network supports at most 241 provider-backed VM workloads.";
        }
        {
          assertion =
            !network.lan.allowEastWest || cfg.site.allowUnsafeEastWest;
          message = "d2b.realms.${realmName}.network.lan.allowEastWest requires d2b.site.allowUnsafeEastWest = true.";
        }
        {
          assertion = network.mtu == null || network.mtu >= 576;
          message = "d2b.realms.${realmName}.network.mtu must be at least 576.";
        }
        {
          assertion =
            !network.externalNetwork.attachment.enable
            || network.externalNetwork.attachment.interface != null;
          message = "d2b.realms.${realmName}.network.externalNetwork.attachment.enable requires attachment.interface.";
        }
        {
          assertion =
            network.externalNetwork.attachment.ipv4.method != "static"
            || network.externalNetwork.attachment.ipv4.address != null;
          message = "d2b.realms.${realmName}.network.externalNetwork.attachment.ipv4.method = \"static\" requires attachment.ipv4.address.";
        }
        {
          assertion =
            !network.externalNetwork.egress.enable
            || network.externalNetwork.attachment.enable;
          message = "d2b.realms.${realmName}.network.externalNetwork.egress.enable requires attachment.enable.";
        }
        {
          assertion =
            network.externalNetwork.portForwards == [ ]
            || network.externalNetwork.attachment.enable;
          message = "d2b.realms.${realmName}.network.externalNetwork.portForwards requires attachment.enable.";
        }
      ]
      ++ lib.imap0
        (index: forward: {
          assertion =
            (forward.workload != null) != (forward.targetIp != null)
            && (forward.workload == null
              || builtins.elem forward.workload workloadNames);
          message = "d2b.realms.${realmName}.network.externalNetwork.portForwards[${toString index}] must select exactly one valid local workload or targetIp.";
        })
        network.externalNetwork.portForwards)
    declaredRealmRows;

  pairwiseCidrAssertions =
    let
      count = builtins.length declaredCidrs;
      pairs = lib.flatten (lib.genList
        (left:
          lib.genList
            (offset: {
              a = builtins.elemAt declaredCidrs left;
              b = builtins.elemAt declaredCidrs (left + offset + 1);
            })
            (count - left - 1))
        count);
    in map
      (pair: {
        assertion = !d2bLib.cidrOverlaps pair.a pair.b;
        message = "d2b realm networks must use disjoint CIDRs; ${pair.a} overlaps ${pair.b}.";
      })
      pairs;
  hostCidrAssertions = lib.flatten (map
    (realm:
      map
        (hostCidr: {
          assertion =
            validIpv4Cidr hostCidr
            && (!validIpv4Cidr realm.addressing.lan.cidr
              || !validIpv4Cidr realm.addressing.uplink.cidr
              || (!d2bLib.cidrOverlaps realm.addressing.lan.cidr hostCidr
                && !d2bLib.cidrOverlaps realm.addressing.uplink.cidr hostCidr));
          message = "d2b host LAN CIDR ${hostCidr} must be valid and disjoint from realm network ${realm.realmPath}.";
        })
        cfg.hostLanCidrs)
    realms);
  externalAttachmentAssertions =
    let
      attachments = lib.filter
        (row: row.interface != null)
        (map
          (realm: {
            realmPath = realm.realmPath;
            interface = realm.guest.externalNetwork.attachment.interface;
          })
          (lib.filter
            (realm: realm.guest.externalNetwork.attachment.enable)
            realms));
      grouped = lib.groupBy (row: row.interface) attachments;
      conflicts =
        lib.filterAttrs (_: rows: builtins.length rows > 1) grouped;
    in lib.mapAttrsToList
      (interface: rows: {
        assertion = false;
        message = "d2b realm networks must not share external attachment ${interface}: ${builtins.toJSON (map (row: row.realmPath) rows)}.";
      })
      conflicts;
in
{
  schemaVersion = "v2";
  inherit realms allocatorRequests;
  assertions =
    perRealmAssertions
    ++ pairwiseCidrAssertions
    ++ hostCidrAssertions
    ++ externalAttachmentAssertions
    ++ [
      {
        assertion = duplicateResourceIds == [ ];
        message = "d2b realm network allocator resource IDs collide: ${builtins.toJSON duplicateResourceIds}.";
      }
      {
        assertion = duplicateIfNames == [ ];
        message = "d2b realm network interface names collide: ${builtins.toJSON duplicateIfNames}.";
      }
    ];
}
