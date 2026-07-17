{ lib, flakeRoot, ... }:

let
  identity = import (flakeRoot + "/nixos-modules/v2-identity.nix");
  stablePrincipalId =
    (import (flakeRoot + "/nixos-modules/lib.nix") { inherit lib; }).stablePrincipalId;
  realmId = path: identity.deriveRealmId path;
  homeId = realmId "home.local-root";
  devId = realmId "dev.local-root";
  workId = realmId "work.local-root";

  fixture =
    { lib, ... }:
    {
      options = {
        assertions = lib.mkOption {
          type = lib.types.listOf lib.types.attrs;
          default = [ ];
        };
        d2b.site = {
          stateDir = lib.mkOption {
            type = lib.types.path;
            default = "/var/lib/d2b";
          };
          adminUsers = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
          };
        };
        d2b._bundle = lib.mkOption {
          type = lib.types.attrs;
          default = { };
        };
        users.groups = lib.mkOption {
          type = lib.types.attrsOf (
            lib.types.submodule {
              options.gid = lib.mkOption {
                type = lib.types.nullOr lib.types.int;
                default = null;
              };
            }
          );
          default = { };
        };
        users.users = lib.mkOption {
          type = lib.types.attrsOf (
            lib.types.submodule {
              options = {
                uid = lib.mkOption {
                  type = lib.types.nullOr lib.types.int;
                  default = null;
                };
                isSystemUser = lib.mkOption {
                  type = lib.types.bool;
                  default = false;
                };
                isNormalUser = lib.mkOption {
                  type = lib.types.bool;
                  default = false;
                };
                group = lib.mkOption {
                  type = lib.types.nullOr lib.types.str;
                  default = null;
                };
                extraGroups = lib.mkOption {
                  type = lib.types.listOf lib.types.str;
                  default = [ ];
                };
                description = lib.mkOption {
                  type = lib.types.str;
                  default = "";
                };
              };
            }
          );
          default = { };
        };
      };

      config = {
        d2b.site.adminUsers = [ "admin" ];
        d2b.realms = {
          local-root = {
            path = "local-root";
            placement = "host-local";
          };
          home = {
            parent = "local-root";
            path = "home.local-root";
            placement = "host-local";
            allowedUsers = [ "alice" ];
            allowedGroups = [ "home-readers" ];
            broker.hostMutation = true;
            providers.runtime = {
              type = "runtime";
              implementationId = "cloud-hypervisor";
            };
            workloads.vm.provider = "runtime";
          };
          dev = {
            parent = "local-root";
            path = "dev.local-root";
            placement = "host-local";
            allowedUsers = [ "bob" ];
          };
          work = {
            parent = "local-root";
            path = "work.local-root";
            placement = "host-local";
            allowedUsers = [ "alice" ];
            keys = {
              realmIdentityRef = "work-identity";
              realmIdentityFingerprint =
                "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
              controllerKeyRef = "work-controller";
              controllerCredentialFingerprint =
                "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
            };
          };
          remote = {
            parent = "local-root";
            path = "remote.local-root";
            placement = "gateway-vm";
          };
        };
        users.users = lib.genAttrs [ "admin" "alice" "bob" ] (_: {
          isNormalUser = true;
        });
      };
    };

  evaluated = lib.evalModules {
    modules = [
      fixture
      (flakeRoot + "/nixos-modules/options-realms.nix")
      (flakeRoot + "/nixos-modules/index.nix")
      (flakeRoot + "/nixos-modules/realm-users.nix")
      (flakeRoot + "/nixos-modules/realm-access.nix")
      (flakeRoot + "/nixos-modules/allocator-json.nix")
      (flakeRoot + "/nixos-modules/realm-controller-config-json.nix")
      (flakeRoot + "/nixos-modules/realm-identity-config-json.nix")
    ];
  };
  cfg = evaluated.config;
  rows = cfg.d2b._realmAllocatorRows;
  endpointsFor = realm: lib.filter (row: row.realmId == realm) rows.endpoints;
  processesFor = realm: lib.filter (row: row.realmId == realm) rows.processes;
  requestFor = realm:
    lib.findFirst (row: row.realmId == realm) null rows.leaseRequests;
  controllerFor = realm:
    lib.findFirst (
      row: row.realmId == realm
    ) null cfg.d2b._bundle.realmControllersJson.data.controllers;
  processFreeKinds =
    map (row: row.kind) (lib.filter (row: row.processFree) rows.cgroups);
  failedAssertions = lib.filter (entry: !entry.assertion) cfg.assertions;
in
{
  "realm-allocator-emission/canonical-child-records" = {
    expr = {
      endpointOrder = map (row: row.endpointId) rows.endpoints;
      endpointCount = builtins.length rows.endpoints;
      remoteEndpointCount = builtins.length (endpointsFor (realmId "remote.local-root"));
      homeEndpoints = map (
        row:
        {
          inherit (row)
            endpointKind
            path
            acceptor
            owner
            group
            mode
            handoffRole
            fdName
            preBindRequired
            systemdActivation
            selfBind
            ;
        }
      ) (endpointsFor homeId);
      homeProcesses = map (
        row:
        {
          inherit (row)
            processRole
            principal
            listenerRef
            cgroupLeaf
            parentSpawnRequired
            initialCgroupPlacement
            receivesSystemdListenFds
            selfBindsListener
            spawnAuthority
            supervisionOwner
            declarativeOnly
            ;
          namespaceCount = builtins.length row.namespaceRefs;
        }
      ) (processesFor homeId);
      requestCounts = map (
        realm: builtins.length (requestFor realm).resources
      ) [
        devId
        homeId
        workId
      ];
      requestIds = map (request: request.requestId) rows.leaseRequests;
      namespaceCount = builtins.length rows.namespaces;
      identityConfigCount = builtins.length rows.identityConfigs;
      homeIdentityConfigs = map (
        row: {
          inherit (row)
            processRole
            principal
            primaryGroup
            supplementaryGroups
            uidMap
            gidMap
            initialNamespaceCapabilitiesEmpty
            ;
        }
      ) (lib.filter (row: row.realmId == homeId) rows.identityConfigs);
      cgroupCount = builtins.length rows.cgroups;
      roleLeafCount =
        builtins.length (lib.filter (row: row.kind == "role-leaf") rows.cgroups);
      roleLeavesAcceptProcesses =
        lib.all (
          row: !row.processFree
        ) (lib.filter (row: row.kind == "role-leaf") rows.cgroups);
      inherit processFreeKinds;
      ownershipRealmIds =
        lib.unique (map (row: row.realmId) rows.ownership);
      allRequestsTyped =
        lib.all (request: request.typed && request.declarativeOnly) rows.leaseRequests;
      allAssertionsPass = failedAssertions == [ ];
      inherit (rows) invariants;
    };
    expected = {
      endpointOrder = [
        "realm-${devId}-public-listener"
        "realm-${devId}-broker-listener"
        "realm-${homeId}-public-listener"
        "realm-${homeId}-broker-listener"
        "realm-${workId}-public-listener"
        "realm-${workId}-broker-listener"
      ];
      endpointCount = 6;
      remoteEndpointCount = 0;
      homeEndpoints = [
        {
          endpointKind = "public";
          path = "/run/d2b/r/${homeId}/public.sock";
          acceptor = "d2bd-r-${homeId}";
          owner = "d2bd-r-${homeId}";
          group = "d2b-r-${homeId}";
          mode = "0660";
          handoffRole = "controller";
          fdName = "public-listener";
          preBindRequired = true;
          systemdActivation = false;
          selfBind = false;
        }
        {
          endpointKind = "broker";
          path = "/run/d2b/r/${homeId}/broker.sock";
          acceptor = "d2bbr-r-${homeId}";
          owner = "d2bbr-r-${homeId}";
          group = "d2bd-r-${homeId}";
          mode = "0660";
          handoffRole = "broker";
          fdName = "broker-listener";
          preBindRequired = true;
          systemdActivation = false;
          selfBind = false;
        }
      ];
      homeProcesses = [
        {
          processRole = "controller";
          principal = "d2bd-r-${homeId}";
          listenerRef = "realm-${homeId}-public-listener";
          cgroupLeaf = "/sys/fs/cgroup/d2b.slice/r-${homeId}/controller";
          parentSpawnRequired = true;
          initialCgroupPlacement = "direct";
          receivesSystemdListenFds = false;
          selfBindsListener = false;
          spawnAuthority = "local-root-broker";
          supervisionOwner = "local-root-controller";
          declarativeOnly = true;
          namespaceCount = 6;
        }
        {
          processRole = "broker";
          principal = "d2bbr-r-${homeId}";
          listenerRef = "realm-${homeId}-broker-listener";
          cgroupLeaf = "/sys/fs/cgroup/d2b.slice/r-${homeId}/broker";
          parentSpawnRequired = true;
          initialCgroupPlacement = "direct";
          receivesSystemdListenFds = false;
          selfBindsListener = false;
          spawnAuthority = "local-root-broker";
          supervisionOwner = "local-root-controller";
          declarativeOnly = true;
          namespaceCount = 6;
        }
      ];
      requestCounts = [ 22 22 22 ];
      requestIds = [
        "realm-${devId}-bootstrap-lease"
        "realm-${homeId}-bootstrap-lease"
        "realm-${workId}-bootstrap-lease"
      ];
      namespaceCount = 36;
      identityConfigCount = 6;
      homeIdentityConfigs = [
        {
          processRole = "controller";
          principal = "d2bd-r-${homeId}";
          primaryGroup = "d2bd-r-${homeId}";
          supplementaryGroups = [ "d2bcg-r-${homeId}" ];
          uidMap = [
            {
              insideId = 0;
              outsideId = stablePrincipalId "d2bd-r-${homeId}";
              length = 1;
            }
          ];
          gidMap = [
            {
              insideId = 0;
              outsideId = stablePrincipalId "d2bd-r-${homeId}";
              length = 1;
            }
            {
              insideId = 1;
              outsideId = stablePrincipalId "d2bcg-r-${homeId}";
              length = 1;
            }
          ];
          initialNamespaceCapabilitiesEmpty = true;
        }
        {
          processRole = "broker";
          principal = "d2bbr-r-${homeId}";
          primaryGroup = "d2bbr-r-${homeId}";
          supplementaryGroups = [ "d2bcg-r-${homeId}" ];
          uidMap = [
            {
              insideId = 0;
              outsideId = stablePrincipalId "d2bbr-r-${homeId}";
              length = 1;
            }
          ];
          gidMap = [
            {
              insideId = 0;
              outsideId = stablePrincipalId "d2bbr-r-${homeId}";
              length = 1;
            }
            {
              insideId = 1;
              outsideId = stablePrincipalId "d2bcg-r-${homeId}";
              length = 1;
            }
          ];
          initialNamespaceCapabilitiesEmpty = true;
        }
      ];
      cgroupCount = 18;
      roleLeafCount = 5;
      roleLeavesAcceptProcesses = true;
      processFreeKinds = [
        "realm-root"
        "workloads-root"
        "realm-root"
        "workloads-root"
        "workload-root"
        "realm-root"
        "workloads-root"
      ];
      ownershipRealmIds = [
        devId
        homeId
        workId
      ];
      allRequestsTyped = true;
      allAssertionsPass = true;
      invariants = {
        declarativeOnly = true;
        childUnitsEmitted = false;
        listenerBindingPerformed = false;
        processSpawnPerformed = false;
        leaseExecutionPerformed = false;
        realmRootsProcessFree = true;
        workloadInteriorsProcessFree = true;
      };
    };
  };

  "realm-allocator-emission/artifact-projections-and-identities" = {
    expr =
      let
        home = controllerFor homeId;
        workIdentity =
          builtins.head cfg.d2b._bundle.realmIdentityJson.data.realms;
      in
      {
        allocatorRealmPaths =
          map (row: row.realmPath) cfg.d2b._bundle.allocatorJson.data.realms;
        allocatorResourceCount =
          builtins.length cfg.d2b._bundle.allocatorJson.data.resourceRequests;
        controllerRealmPaths =
          map (row: row.realmPath) cfg.d2b._bundle.realmControllersJson.data.controllers;
        homeIdentity = {
          controller = home.daemon.user;
          broker = home.broker.user;
          publicGroup = home.daemon.publicSocketGroup;
          publicSocket = home.sockets.publicSocketPath;
          brokerSocket = home.sockets.brokerSocketPath;
          resourceCount = builtins.length home.allocator.resourceRequestRefs;
          daemonMaterialized = home.daemon.materializedService;
          brokerSocketMaterialized = home.broker.materializedSocket;
          brokerServiceMaterialized = home.broker.materializedService;
        };
        inherit workIdentity;
        controllerInvariants =
          cfg.d2b._bundle.realmControllersJson.data.invariants;
      };
    expected = {
      allocatorRealmPaths = [
        "dev.local-root"
        "home.local-root"
        "work.local-root"
      ];
      allocatorResourceCount = 66;
      controllerRealmPaths = [
        "dev.local-root"
        "home.local-root"
        "work.local-root"
      ];
      homeIdentity = {
        controller = "d2bd-r-${homeId}";
        broker = "d2bbr-r-${homeId}";
        publicGroup = "d2b-r-${homeId}";
        publicSocket = "/run/d2b/r/${homeId}/public.sock";
        brokerSocket = "/run/d2b/r/${homeId}/broker.sock";
        resourceCount = 22;
        daemonMaterialized = false;
        brokerSocketMaterialized = false;
        brokerServiceMaterialized = false;
      };
      workIdentity = {
        realm = [
          "work"
          "local-root"
        ];
        realmIdentityRef = "work-identity";
        realmIdentityFingerprint =
          "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        controllerCredentialRef = "work-controller";
        controllerCredentialFingerprint =
          "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
      };
      controllerInvariants = {
        metadataOnly = true;
        noSystemdUnitsMaterialized = true;
        preservesGlobalDaemonBehavior = true;
        preservesDirectUnixSocketSemantics = true;
      };
    };
  };
}
