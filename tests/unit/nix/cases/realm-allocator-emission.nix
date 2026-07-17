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
        d2b._index.devices.allocatorLeaseRequests = [
          {
            realmPath = "home.local-root";
            resourceId = "device-tpm-${homeId}";
            kind = "host-file-partition";
            share = "exclusive";
            source = {
              kind = "realm-broker";
              refName = "provider-device-home";
            };
            acquisitionOrder = {
              phase = 50;
              ordinal = 0;
            };
          }
        ];
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
            workloads.vm.providerRefs.runtime = "runtime";
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
  allocatorData = cfg.d2b._bundle.allocatorJson.data;
  launchFor = realm:
    lib.findFirst (row: row.realmId == realm) null allocatorData.processLaunch;
  launchDigest =
    row:
    "sha256:${builtins.hashString "sha256" (
      builtins.toJSON (builtins.removeAttrs row [ "launchRecordDigest" ])
    )}";
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
      allocatorResourceCount = 67;
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

  "realm-allocator-emission/typed-launch-authority" = {
    expr =
      let
        home = launchFor homeId;
        tampered = home // {
          controller = home.controller // {
            uid = home.controller.uid + 1;
          };
        };
        childKeys = builtins.attrNames home.controller;
        opaqueRefs =
          [
            home.controllerGeneration
            home.controller.processId
            home.controller.configRef
            home.controller.listenerRef
            home.controller.bootstrapSessionRef
            home.controller.cgroupRef
            home.controller.stateRootRef
            home.controller.auditRootRef
          ]
          ++ home.controller.resourceRefs
          ++ home.controller.leaseRefs
          ++ map (namespace: namespace.refId) (
            builtins.attrValues home.controller.namespaces
          );
        digests =
          [
            home.launchRecordDigest
            home.controller.executableDigest
            home.controller.configDigest
            home.controller.cgroupDigest
            home.broker.executableDigest
            home.broker.configDigest
            home.broker.cgroupDigest
          ]
          ++ map (namespace: namespace.digest) (
            builtins.attrValues home.controller.namespaces
            ++ builtins.attrValues home.broker.namespaces
          );
        homeResourceIds = map (row: row.resourceId) (
          lib.filter (
            row: row.realmPath == "home.local-root"
          ) allocatorData.resourceRequests
        );
        deviceRef = "device-tpm-${homeId}";
      in
      {
        topLevelKeys = builtins.attrNames home;
        inherit childKeys;
        namespaceKeys = builtins.attrNames home.controller.namespaces;
        spawnKeys = builtins.attrNames home.controller.spawn;
        launchOrder = map (
          row: "${row.realmId}:${row.controllerGeneration}"
        ) allocatorData.processLaunch;
        launchCount = builtins.length allocatorData.processLaunch;
        homeIdentity = {
          controller = {
            inherit (home.controller) role processId uid gid;
          };
          broker = {
            inherit (home.broker) role processId uid gid;
          };
        };
        homeRefs = {
          controllerListener = home.controller.listenerRef;
          brokerListener = home.broker.listenerRef;
          controllerCgroup = home.controller.cgroupRef;
          brokerCgroup = home.broker.cgroupRef;
          controllerNamespaceCount =
            builtins.length (builtins.attrValues home.controller.namespaces);
          brokerNamespaceCount =
            builtins.length (builtins.attrValues home.broker.namespaces);
        };
        executableRefs = [
          home.controller.executableRef
          home.broker.executableRef
        ];
        allSpawnAuthorityClosed =
          lib.all (value: value) (builtins.attrValues home.controller.spawn)
          && lib.all (value: value) (builtins.attrValues home.broker.spawn);
        refsSorted =
          home.controller.resourceRefs
          == lib.sort lib.lessThan home.controller.resourceRefs
          && home.controller.leaseRefs
          == lib.sort lib.lessThan home.controller.leaseRefs
          && home.broker.resourceRefs
          == lib.sort lib.lessThan home.broker.resourceRefs
          && home.broker.leaseRefs
          == lib.sort lib.lessThan home.broker.leaseRefs;
        refsUnique =
          lib.all (
            refs: builtins.length refs == builtins.length (lib.unique refs)
          ) [
            home.controller.resourceRefs
            home.controller.leaseRefs
            home.broker.resourceRefs
            home.broker.leaseRefs
          ];
        launchDigestsMatch =
          lib.all (row: row.launchRecordDigest == launchDigest row) (
            allocatorData.processLaunch
          );
        tamperChangesDigest = home.launchRecordDigest != launchDigest tampered;
        digestShape =
          lib.all (
            digest:
            builtins.substring 0 7 digest == "sha256:"
            && builtins.stringLength digest == 71
          ) digests;
        opaqueRefsPathFree = lib.all (ref: !(lib.hasInfix "/" ref)) opaqueRefs;
        noAmbientLaunchFields =
          lib.all (
            key: !(builtins.elem key childKeys)
          ) [
            "argv"
            "environment"
            "path"
            "uidMap"
            "gidMap"
          ];
        deviceRequestEmitted = builtins.elem deviceRef homeResourceIds;
        deviceLeaseResolves =
          builtins.elem deviceRef home.broker.resourceRefs
          && builtins.elem deviceRef home.broker.leaseRefs;
        controllerHasNoDeviceLease =
          !(builtins.elem deviceRef home.controller.resourceRefs)
          && !(builtins.elem deviceRef home.controller.leaseRefs);
        allAssertionsPass = failedAssertions == [ ];
      };
    expected = {
      topLevelKeys = [
        "broker"
        "controller"
        "controllerGeneration"
        "launchRecordDigest"
        "realmId"
        "realmPath"
      ];
      childKeys = [
        "auditRootRef"
        "bootstrapSessionRef"
        "cgroupDigest"
        "cgroupRef"
        "configDigest"
        "configRef"
        "executableDigest"
        "executableRef"
        "gid"
        "leaseRefs"
        "listenerRef"
        "namespaces"
        "processId"
        "resourceRefs"
        "role"
        "spawn"
        "stateRootRef"
        "uid"
      ];
      namespaceKeys = [
        "cgroup"
        "ipc"
        "mount"
        "network"
        "pid"
        "user"
      ];
      spawnKeys = [
        "clone3WithPidfd"
        "closedEnvironment"
        "directCgroupPlacement"
        "emptyInitialCapabilities"
        "executableOnlyArgv"
        "inheritedFdAuthorityOnly"
        "noNewPrivileges"
      ];
      launchOrder = [
        "${workId}:realm-${workId}-controller-generation"
        "${devId}:realm-${devId}-controller-generation"
        "${homeId}:realm-${homeId}-controller-generation"
      ];
      launchCount = 3;
      homeIdentity = {
        controller = {
          role = "controller";
          processId = "realm-${homeId}-controller";
          uid = stablePrincipalId "d2bd-r-${homeId}";
          gid = stablePrincipalId "d2bd-r-${homeId}";
        };
        broker = {
          role = "broker";
          processId = "realm-${homeId}-broker";
          uid = stablePrincipalId "d2bbr-r-${homeId}";
          gid = stablePrincipalId "d2bbr-r-${homeId}";
        };
      };
      homeRefs = {
        controllerListener = "realm-${homeId}-public-listener";
        brokerListener = "realm-${homeId}-broker-listener";
        controllerCgroup = "realm-${homeId}-cgroup-controller";
        brokerCgroup = "realm-${homeId}-cgroup-broker";
        controllerNamespaceCount = 6;
        brokerNamespaceCount = 6;
      };
      executableRefs = [
        "/run/current-system/sw/bin/d2bd"
        "/run/current-system/sw/bin/d2b-priv-broker"
      ];
      allSpawnAuthorityClosed = true;
      refsSorted = true;
      refsUnique = true;
      launchDigestsMatch = true;
      tamperChangesDigest = true;
      digestShape = true;
      opaqueRefsPathFree = true;
      noAmbientLaunchFields = true;
      deviceRequestEmitted = true;
      deviceLeaseResolves = true;
      controllerHasNoDeviceLease = true;
      allAssertionsPass = true;
    };
  };
}
