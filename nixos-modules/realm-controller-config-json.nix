{ config, lib, ... }:

let
  cfg = config.d2b;
  guestSessionRows = cfg._workloadGuestSessionCredentialRows or [ ];
  allocatorRows = cfg._realmAllocatorRows;
  childRealms = lib.sortOn (row: row.realmPath) (cfg._realmAccess.children or [ ]);
  allocatorData = cfg._bundle.allocatorJson.data;

  processFor =
    realmId: processRole:
    lib.findFirst (
      process: process.realmId == realmId && process.processRole == processRole
    ) (throw "missing ${processRole} process row for realm ${realmId}") allocatorRows.processes;

  endpointFor =
    realmId: endpointKind:
    lib.findFirst (
      endpoint: endpoint.realmId == realmId && endpoint.endpointKind == endpointKind
    ) (throw "missing ${endpointKind} endpoint row for realm ${realmId}") allocatorRows.endpoints;

  providersFor =
    realmId:
    map (
      provider: {
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
      lib.sortOn (provider: provider.providerId) (
        lib.filter (
          provider: provider.realmId == realmId
        ) cfg._index.providers.enabledList
      )
    );

  providerPlacementFor =
    row:
    let
      declared = cfg.realms.${cfg._index.realms.byId.${row.realmId}.realmName};
      matches = lib.filter (
        provider: provider.providerName == declared.placementProvider
      ) cfg._index.providers.enabledList;
      provider = if matches == [ ] then null else builtins.head matches;
    in
    if provider == null
    then null
    else {
      inherit (provider) providerName providerId;
      kind = provider.providerType;
      providerSpecificPlacement = declared.providerSpecificPlacement;
    };

  resourceRefsFor =
    realmId:
    map (resource: resource.resourceId) (
      lib.filter (resource: resource.realmId == realmId) allocatorRows.resources
    );

  controllerConfig =
    row:
    let
      realm = cfg._index.realms.byId.${row.realmId};
      declared = cfg.realms.${realm.realmName};
      controller = processFor row.realmId "controller";
      broker = processFor row.realmId "broker";
      publicEndpoint = endpointFor row.realmId "public";
      brokerEndpoint = endpointFor row.realmId "broker";
    in
    {
      inherit (realm)
        realmName
        realmId
        realmPath
        placement
        ;
      providerPlacement = providerPlacementFor row;
      daemon = {
        user = controller.principal;
        group = controller.group;
        publicSocketGroup = row.publicGroup;
        serviceName = controller.launchId;
        configPath = controller.configRef;
        stateLockPath = "${row.resources.runtime.path}/locks/controller.lock";
        locksDir = "${row.resources.runtime.path}/locks";
        socketActivated = false;
        materializedService = false;
      };
      broker = {
        enabled = true;
        user = broker.principal;
        group = broker.group;
        socketUnitName = brokerEndpoint.endpointId;
        serviceUnitName = broker.launchId;
        socketPath = brokerEndpoint.path;
        auditDir = row.resources.audit.path;
        hostMutation = declared.broker.hostMutation;
        materializedSocket = false;
        materializedService = false;
      };
      paths = {
        runDir = row.resources.runtime.path;
        stateDir = row.resources.state.path;
        auditDir = row.resources.audit.path;
      };
      sockets = {
        publicSocketPath = publicEndpoint.path;
        brokerSocketPath = brokerEndpoint.path;
      };
      allocator = {
        kind = "local-root-metadata";
        configPath = "/etc/d2b/allocator.json";
        rootSocket = allocatorData.allocator.rootSocket;
        resourceRequestRefs = resourceRefsFor row.realmId;
      };
      access = {
        allowedUsers = row.allowedUsers;
        allowedGroups = row.allowedGroups;
        inheritedAdminUsers = lib.sort lib.lessThan (lib.unique cfg.site.adminUsers);
      };
      localRuntime = null;
      providers = providersFor row.realmId;
    };

  data = {
    schemaVersion = "v2";
    runtimeState = "metadata-only";
    controllers = map controllerConfig childRealms;
    invariants = {
      metadataOnly = true;
      noSystemdUnitsMaterialized = true;
      preservesGlobalDaemonBehavior = true;
      preservesDirectUnixSocketSemantics = true;
    };
  };
  dataJson = builtins.toJSON data;
in
{
  config = {
    assertions = [
      {
        assertion = lib.all (
          controller:
          builtins.substring 0 9 controller.daemon.stateLockPath == "/run/d2b/"
          && builtins.substring 0 9 controller.daemon.locksDir == "/run/d2b/"
        ) data.controllers;
        message = "realm controller lock metadata must remain under /run/d2b";
      }
      {
        assertion = lib.all
          (credential:
            credential.authority.generation
              == "d2bd-r-${credential.realmId}"
            && credential.authority.materialization
              == "d2bbr-r-${credential.realmId}")
          guestSessionRows;
        message = "realm guest session credential authority must remain child-realm confined";
      }
      {
        assertion =
          !(lib.hasInfix "GuestSessionCredentialV1" dataJson)
          && !(lib.hasInfix "d2b-guest-session-v2" dataJson)
          && !(lib.hasInfix "parentPrivateKey" dataJson)
          && !(lib.hasInfix "guestPrivateKey" dataJson)
          && !(lib.hasInfix "operationPsk" dataJson);
        message = "runtime guest session credentials must not enter realm-controllers.json";
      }
    ];

    d2b._bundle.realmControllersJson = {
      inherit data;
      installFileName = "realm-controllers.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };
}
