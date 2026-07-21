{ lib, flakeRoot, ... }:

let
  identity = import (flakeRoot + "/nixos-modules/v2-identity.nix");
  mkRealm = id: lanSubnet: uplinkSubnet: {
    enable = true;
    inherit id;
    path = id;
    placement = "host-local";
    broker = {
      enable = true;
      hostMutation = true;
    };
    network = {
      mode = "declared";
      inherit lanSubnet uplinkSubnet;
      mtu = null;
      mssClamp = false;
      netVmName = null;
      lan.allowEastWest = false;
      hostBlocklist = [ "169.254.0.0/16" ];
      externalNetwork = {
        enable = false;
        attachment = {
          enable = false;
          interface = null;
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
          enable = false;
          allowedCidrs = [ ];
          masquerade = true;
        };
        portForwards = [ ];
        mdns = {
          enable = false;
          reflector.enable = true;
          dnsmasqLocal = {
            enable = false;
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
  mkRealmRow = id:
    let realmId = identity.deriveRealmId "${id}.local-root";
    in {
      inherit realmId;
      realmName = id;
      realmPath = "${id}.local-root";
      placement = "host-local";
      metadata.configuredId = id;
    };
  mkWorkloadRow = id:
    let
      realmId = identity.deriveRealmId "${id}.local-root";
      workloadId = identity.deriveWorkloadId realmId "app";
    in {
      enabled = true;
      inherit realmId workloadId;
      realmName = id;
      realmPath = "${id}.local-root";
      workloadName = "app";
      canonicalTarget = "app.${id}.local-root.d2b";
      providerBindings.runtime = {
        implementationId = "cloud-hypervisor";
        providerId = identity.deriveProviderId realmId "runtime" "vm";
        providerType = "runtime";
      };
    };

  config.d2b = {
    site.allowUnsafeEastWest = false;
    hostLanCidrs = [ "192.168.0.0/16" ];
    realms = {
      work = mkRealm "work" "10.20.0.0/24" "192.0.2.0/30";
      personal =
        mkRealm "personal" "10.30.0.0/24" "198.51.100.0/30";
    };
    _index = {
      realms.enabledList = map mkRealmRow [ "personal" "work" ];
      workloads.enabledByRealmId = lib.listToAttrs (map
        (id:
          let row = mkWorkloadRow id;
          in {
            name = row.realmId;
            value = [ row ];
          })
        [ "personal" "work" ]);
    };
  };
  plan = import (flakeRoot + "/nixos-modules/realm-network-rows.nix") {
    inherit config lib;
  };
  overlapPlan = import (flakeRoot + "/nixos-modules/realm-network-rows.nix") {
    inherit lib;
    config.d2b = config.d2b // {
      realms = config.d2b.realms // {
        personal =
          mkRealm "personal" "10.20.0.0/24" "198.51.100.0/30";
      };
    };
  };
  byName = lib.listToAttrs (map
    (realm: {
      name = realm.realmName;
      value = realm;
    })
    plan.realms);
  resourceIds = map (request: request.resourceId) plan.allocatorRequests;
  nftChainNames = lib.flatten (map
    (realm: map (chain: chain.name) realm.resources.nftables.chains)
    plan.realms);
in
{
  "multi-env-daemon/two-realm-network-rows" = {
    expr = map (realm: realm.realmName) plan.realms;
    expected = [ "personal" "work" ];
  };

  "multi-env-daemon/disjoint-lan-addresses" = {
    expr = {
      personal = (builtins.head byName.personal.addressing.workloadRows).ip;
      work = (builtins.head byName.work.addressing.workloadRows).ip;
    };
    expected = {
      personal = "10.30.0.10";
      work = "10.20.0.10";
    };
  };

  "multi-env-daemon/default-isolation-per-realm" = {
    expr = map
      (realm: realm.policy.workloadTapIsolation)
      plan.realms;
    expected = [ true true ];
  };

  "multi-env-daemon/resource-ids-unique" = {
    expr = builtins.length resourceIds
      == builtins.length (lib.unique resourceIds);
    expected = true;
  };

  "multi-env-daemon/nft-chain-names-unique" = {
    expr = builtins.length nftChainNames
      == builtins.length (lib.unique nftChainNames);
    expected = true;
  };

  "multi-env-daemon/cross-realm-ifnames-unique" = {
    expr =
      let
        names = lib.flatten (map
          (realm: realm.coexistence.networkManager.unmanagedIfNames)
          plan.realms);
      in builtins.length names == builtins.length (lib.unique names);
    expected = true;
  };

  "multi-env-daemon/all-assertions-pass" = {
    expr = builtins.all (assertion: assertion.assertion) plan.assertions;
    expected = true;
  };

  "multi-env-daemon/overlapping-realms-fail-closed" = {
    expr =
      builtins.any (assertion: !assertion.assertion)
        overlapPlan.assertions;
    expected = true;
  };
}
