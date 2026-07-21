{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  networkPlan = import ./realm-network-rows.nix {
    inherit config lib;
  };

  moduleRow = module: feature: requirement: gate: sysctls: jailVisibleDevice: {
    inherit module feature requirement gate sysctls jailVisibleDevice;
  };

  fdRow = resource: brokerOperation: recipient: transfer: jailVisibleDevice: notes: {
    inherit resource brokerOperation recipient transfer jailVisibleDevice notes;
  };

  bridgeFlags = realm: [
    {
      role = "net-vm-lan";
      isolated = false;
      neighSuppress = false;
      learning = true;
      unicastFlood = true;
      rule = "The realm network workload remains reachable from workload TAPs.";
    }
    {
      role = "workload-lan";
      isolated = realm.policy.workloadTapIsolation;
      neighSuppress = realm.policy.workloadTapIsolation;
      learning = true;
      unicastFlood = !realm.policy.workloadTapIsolation;
      rule = "Workload TAP isolation follows the realm's explicit east-west policy.";
    }
    {
      role = "uplink";
      isolated = true;
      neighSuppress = true;
      learning = false;
      unicastFlood = false;
      rule = "The realm uplink remains point-to-point.";
    }
  ];

  ipv6Row = ifName: {
    inherit ifName;
    disableIpv6 = 1;
    acceptRa = 0;
    autoconf = 0;
    addrGenMode = 1;
    arpIgnore = 1;
  };

  environmentRow = realm: {
    env = realm.canonicalRealmId;
    bridge = realm.resources.bridges.lan.ifName;
    hostUplinkIp = realm.addressing.uplink.host;
    netUplinkIp = realm.addressing.uplink.netVm;
    mtu = if realm.policy.mtu == null then 1500 else realm.policy.mtu;
    mssClamp =
      if realm.policy.mssClamp
      then (if realm.policy.mssClampBytes == null
        then 1460
        else realm.policy.mssClampBytes)
      else null;
    lan = {
      allowEastWest = realm.policy.allowEastWest;
      effectiveEastWest = realm.policy.allowEastWest;
    };
    netVmForwardBlocklist = realm.policy.hostBlocklist;
    externalNetwork = null;
    bridgePortFlags = bridgeFlags realm;
    ipv6Sysctls = map ipv6Row
      realm.coexistence.networkManager.unmanagedIfNames;
    usbipBusidLocks = [ ];
  };

  mappingRows = realm:
    [
      {
        env = realm.canonicalRealmId;
        role = "net-vm-lan";
        userVisibleName = realm.resources.bridges.lan.ifName;
        derivedIfname = realm.resources.bridges.lan.ifName;
      }
      {
        env = realm.canonicalRealmId;
        role = "uplink";
        userVisibleName = realm.resources.bridges.uplink.ifName;
        derivedIfname = realm.resources.bridges.uplink.ifName;
      }
    ]
    ++ map (workload: {
      env = realm.canonicalRealmId;
      vm = workload.canonicalWorkloadId;
      role = "workload-lan";
      userVisibleName = workload.tap.ifName;
      derivedIfname = workload.tap.ifName;
    }) realm.addressing.workloadRows;

  realms = lib.sortOn (realm: realm.canonicalRealmId) networkPlan.realms;
  realmIds = map (realm: realm.canonicalRealmId) realms;
  managedIfNames = lib.unique (lib.flatten
    (map (realm: realm.coexistence.networkManager.unmanagedIfNames) realms));
  ownershipId = "d2b-${
    builtins.substring 0 8
      (builtins.hashString "sha256" (builtins.concatStringsSep "," realmIds))
  }";

  data = {
    schemaVersion = "v2";
    site.allowUnsafeEastWest = cfg.site.allowUnsafeEastWest;
    environments = map environmentRow realms;
    nftables = {
      family = "inet";
      table = "d2b";
      inherit ownershipId;
      tableHashAfterApply = null;
      chains = [
        {
          name = "prerouting";
          hook = "prerouting";
          priority = -150;
          policy = "accept";
          purpose = "Realm classification before forwarding.";
        }
        {
          name = "forward";
          hook = "forward";
          priority = -5;
          policy = "drop";
          purpose = "Default-deny realm forwarding.";
        }
        {
          name = "output";
          hook = "output";
          priority = -5;
          policy = "accept";
          purpose = "Host-originated realm traffic.";
        }
        {
          name = "input";
          hook = "input";
          priority = -5;
          policy = "accept";
          purpose = "Broker-mediated host ingress.";
        }
      ];
    };
    networkManager = {
      filePath = "/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf";
      matchCriteria = map (ifName: "interface-name:${ifName}") managedIfNames;
      reloadBehavior = "Reload NetworkManager after replacing the marked d2b unmanaged-device file.";
      ownership = {
        owner = "root";
        group = "root";
        mode = "0644";
        driftPolicy = "Replace only the d2b-managed file and preserve foreign configuration.";
      };
    };
    hostsFile = {
      startMarker = "# d2b-managed begin";
      endMarker = "# d2b-managed end";
      rule = "Replace only the deterministic marked block and preserve foreign lines.";
    };
    kernelModules = [
      (moduleRow "kvm" "realm-workload-runtime" "required"
        "Local VM workloads require KVM." [ ] false)
      (moduleRow "tun" "realm-network-tap" "required"
        "Declared realm networks use broker-created TAP descriptors." [ ] false)
      (moduleRow "vhost_net" "realm-network-vhost" "required"
        "Local VM workload networking uses vhost-net." [ ] false)
      (moduleRow "fuse" "realm-store-view" "required"
        "Workload store views use virtiofs." [ ] false)
      (moduleRow "nf_tables" "realm-firewall" "required"
        "Realm network partitions use nftables." [ ] false)
      (moduleRow "bridge" "realm-bridge" "required"
        "Declared realm networks use Linux bridges." [ ] false)
    ];
    fdOwnership = [
      (fdRow "/dev/kvm" "OpenKvm" "workload-runtime" "SCM_RIGHTS" false
        "The broker passes a KVM descriptor to the selected runtime role.")
      (fdRow "tap" "CreateTapFd" "workload-runtime" "SCM_RIGHTS" false
        "The allocator and realm broker own TAP creation.")
      (fdRow "/dev/vhost-net" "OpenVhostNet" "workload-runtime" "SCM_RIGHTS" false
        "The broker transfers vhost-net with the TAP descriptor.")
      (fdRow "/dev/fuse" "OpenFuse" "virtiofsd" "SCM_RIGHTS" false
        "The broker transfers only the declared FUSE descriptor.")
      (fdRow "cgroup-dirfd" "OpenCgroupDir" "realm-controller" "SCM_RIGHTS" false
        "Controllers receive only their delegated cgroup partition.")
    ];
    runtimeProviders = [ ];
    vmRuntimes = [ ];
    qemuMedia = null;
    cloudHypervisorCapabilities = [ ];
    ifNameMappings = lib.flatten (map mappingRows realms);
    ch.netHandoffMode = cfg.site.ch.netHandoffMode;
    firewallCoexistencePolicy = {
      manager = "none";
      policy = "coexist";
      rationale = "The broker preserves foreign rules and owns only marked realm partitions.";
    };
  };

  jsonText = builtins.toJSON data;
in
{
  config.d2b._bundle.hostJson = {
    inherit data jsonText;
    path = "${pkgs.writeText "d2b-host.json" jsonText}";
    installFileName = "host.json";
    classification = "contractPrivateNonSecret";
    sensitivity = "nonSecret";
  };
}
