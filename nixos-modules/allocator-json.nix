{ config, lib, ... }:

let
  cfg = config.d2b;
  allocatorRows = cfg._realmAllocatorRows;
  deviceResourceRequests =
    (cfg._index.devices or { }).allocatorLeaseRequests or [ ];
  childRealms = lib.sortOn (row: row.realmPath) (cfg._realmAccess.children or [ ]);
  childRealmIds = map (row: row.realmId) childRealms;
  allocatorStateDir = "${toString cfg.site.stateDir}/allocator";
  allocatorRunDir = "/run/d2b/allocator";
  allocatorRootSocket = "${allocatorRunDir}/local-root.sock";

  declaredRealm = row: cfg.realms.${cfg._index.realms.byId.${row.realmId}.realmName};
  indexRealm = row: cfg._index.realms.byId.${row.realmId};

  realmMetadata =
    row:
    let
      realm = indexRealm row;
      declared = declaredRealm row;
    in
    {
      inherit (realm)
        realmName
        realmId
        realmPath
        placement
        ;
      enabled = true;
      hostMutation = declared.broker.hostMutation;
      placementProvider = declared.placementProvider;
      providerSpecificPlacement = declared.providerSpecificPlacement;
      providerKeys = lib.sort lib.lessThan (lib.attrNames declared.providers);
      envNames = [ ];
    };

  pathPartition = row: {
    realmPath = row.realmPath;
    stateDir = row.resources.state.path;
    runDir = row.resources.runtime.path;
    auditDir = row.resources.audit.path;
    publicSocket = row.resources.publicSocket.path;
    brokerSocket = row.resources.brokerSocket.path;
  };

  resourceRequest = row: {
    inherit (row)
      realmPath
      resourceId
      kind
      share
      acquisitionOrder
      source
      ;
  };
  resourceRequests = lib.sort (
    left: right:
    if left.realmPath != right.realmPath then
      lib.lessThan left.realmPath right.realmPath
    else if left.acquisitionOrder.phase != right.acquisitionOrder.phase then
      left.acquisitionOrder.phase < right.acquisitionOrder.phase
    else if left.acquisitionOrder.ordinal != right.acquisitionOrder.ordinal then
      left.acquisitionOrder.ordinal < right.acquisitionOrder.ordinal
    else if left.kind != right.kind then
      lib.lessThan left.kind right.kind
    else
      lib.lessThan left.resourceId right.resourceId
  ) (
    map resourceRequest allocatorRows.resources
    ++ deviceResourceRequests
  );

  digestJson = value: "sha256:${builtins.hashString "sha256" (builtins.toJSON value)}";
  digestString = value: "sha256:${builtins.hashString "sha256" value}";
  sortedUnique = values: lib.sort lib.lessThan (lib.unique values);
  findRequired =
    description: predicate: rows:
    lib.findFirst predicate (throw "missing ${description}") rows;
  deviceResourcesFor =
    realmPath:
    lib.filter (request: request.realmPath == realmPath) deviceResourceRequests;
  processFor =
    realmId: role:
    findRequired "allocator ${role} process row for realm ${realmId}" (
      row: row.realmId == realmId && row.processRole == role
    ) allocatorRows.processes;
  cgroupFor =
    realmId: role:
    findRequired "allocator ${role} cgroup row for realm ${realmId}" (
      row: row.realmId == realmId && row.kind == "${role}-leaf"
    ) allocatorRows.cgroups;
  namespaceFor =
    realmId: role: kind:
    findRequired "allocator ${role} ${kind} namespace row for realm ${realmId}" (
      row:
      row.realmId == realmId
      && row.processRole == role
      && row.namespaceKind == kind
    ) allocatorRows.namespaces;
  identityFor =
    realmId: role:
    findRequired "allocator ${role} identity row for realm ${realmId}" (
      row: row.realmId == realmId && row.processRole == role
    ) allocatorRows.identityConfigs;
  namespaceAuthorityFor =
    realmId: role: kind:
    let
      row = namespaceFor realmId role kind;
    in
    {
      refId = row.namespaceId;
      digest = digestJson row;
    };
  spawnAuthority = {
    clone3WithPidfd = true;
    directCgroupPlacement = true;
    noNewPrivileges = true;
    emptyInitialCapabilities = true;
    executableOnlyArgv = true;
    closedEnvironment = true;
    inheritedFdAuthorityOnly = true;
  };
  childLaunchFor =
    realm: role:
    let
      process = processFor realm.realmId role;
      cgroup = cgroupFor realm.realmId role;
      identity = identityFor realm.realmId role;
      isController = role == "controller";
      configRef = "realm-${realm.realmId}-${role}-config";
      executableRef =
        if isController
        then "/run/current-system/sw/bin/d2bd"
        else "/run/current-system/sw/bin/d2b-priv-broker";
      deviceRefs = map (request: request.resourceId) (
        deviceResourcesFor realm.realmPath
      );
      resourceRefs = sortedUnique (
        process.resourceRefs ++ lib.optionals (!isController) deviceRefs
      );
      leaseRefs = sortedUnique (
        [ "realm-${realm.realmId}-bootstrap-lease" ]
        ++ lib.optionals (!isController) deviceRefs
      );
      namespaces = {
        user = namespaceAuthorityFor realm.realmId role "user";
        mount = namespaceAuthorityFor realm.realmId role "mount";
        network = namespaceAuthorityFor realm.realmId role "network";
        ipc = namespaceAuthorityFor realm.realmId role "ipc";
        pid = namespaceAuthorityFor realm.realmId role "pid";
        cgroup = namespaceAuthorityFor realm.realmId role "cgroup";
      };
      configAuthority = {
        inherit configRef;
        identityConfigId = identity.identityConfigId;
        listenerRef = process.listenerRef;
        inherit resourceRefs leaseRefs;
      };
    in
    {
      inherit role executableRef configRef resourceRefs leaseRefs namespaces;
      processId = "realm-${realm.realmId}-${role}";
      executableDigest = digestString executableRef;
      configDigest = digestJson configAuthority;
      uid = config.users.users.${process.principal}.uid;
      gid = config.users.groups.${process.group}.gid;
      listenerRef = process.listenerRef;
      bootstrapSessionRef = "realm-${realm.realmId}-${role}-bootstrap";
      cgroupRef = "realm-${realm.realmId}-cgroup-${role}";
      cgroupDigest = digestJson cgroup;
      stateRootRef = "realm-${realm.realmId}-state";
      auditRootRef = "realm-${realm.realmId}-audit";
      spawn = spawnAuthority;
    };
  processLaunch = map (
    realm:
    let
      controllerGeneration = "realm-${realm.realmId}-controller-generation";
      controller = childLaunchFor realm "controller";
      broker = childLaunchFor realm "broker";
      material = {
        inherit (realm) realmId realmPath;
        inherit controllerGeneration controller broker;
      };
    in
    material // {
      launchRecordDigest = digestJson material;
    }
  ) (lib.sortOn (row: row.realmId) childRealms);

  providerPlacements =
    map (
      provider:
      {
        realmPath = cfg._index.realms.byId.${provider.realmId}.realmPath;
        inherit (provider)
          providerName
          providerId
          enabled
          placement
          capabilityRefs
          configRef
          ;
        kind = provider.providerType;
      }
    ) (
      lib.sortOn (provider: "${provider.realmId}/${provider.providerId}") (
        lib.filter (
          provider: builtins.elem provider.realmId childRealmIds
        ) cfg._index.providers.enabledList
      )
    );

  data = {
    schemaVersion = "v2";
    allocator = {
      enabled = childRealms != [ ];
      runtimeState = "metadata-only";
      rootSocket = allocatorRootSocket;
      stateDir = allocatorStateDir;
      leaseLedger = "${allocatorStateDir}/leases.jsonl";
      auditDir = "${allocatorStateDir}/audit";
      runtime = {
        spawnsService = false;
        socketActivated = false;
        serviceName = null;
      };
    };
    realms = map realmMetadata childRealms;
    inherit resourceRequests processLaunch;
    pathPartitions = map pathPartition childRealms;
    inherit providerPlacements;
    envBridge = [ ];
    invariants = {
      noRuntimeAllocatorService = true;
      preservesEnvRuntimeSourceOfTruth = true;
      privateMetadataOnly = true;
    };
  };
in
{
  imports = [ ./realm-allocator-rows.nix ];

  config = {
    assertions = [
      {
        assertion = builtins.substring 0 9 allocatorRootSocket == "/run/d2b/";
        message = "d2b allocator.json rootSocket must remain under /run/d2b";
      }
      {
        assertion = builtins.stringLength allocatorRootSocket <= 107;
        message = "d2b allocator.json rootSocket must fit Linux AF_UNIX sockaddr_un.sun_path";
      }
      {
        assertion =
          builtins.length data.resourceRequests
          == builtins.length (
            lib.unique (map (row: "${row.realmPath}:${row.resourceId}") data.resourceRequests)
          );
        message = "d2b allocator.json realm-scoped resource request IDs must be unique";
      }
      {
        assertion = builtins.length processLaunch <= 64;
        message = "d2b allocator.json processLaunch exceeds its 64-row bound";
      }
    ] ++ lib.concatMap (
      row:
      map (
        child:
        {
          assertion =
            builtins.length child.resourceRefs <= 32
            && builtins.length child.leaseRefs <= 32;
          message =
            "d2b allocator process launch authority for realm "
            + "${row.realmPath} exceeds the 32-reference bound";
        }
      ) [
        row.controller
        row.broker
      ]
    ) processLaunch;

    d2b._bundle.allocatorJson = {
      inherit data;
      installFileName = "allocator.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };
}
