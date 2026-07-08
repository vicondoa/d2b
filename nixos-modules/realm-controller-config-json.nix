{ config, lib, ... }:

let
  cfg = config.d2b;

  sortNames = names: lib.sort lib.lessThan names;
  sortedAttrNames = attrs: sortNames (lib.attrNames attrs);
  sortedMapAttrsToList = f: attrs:
    map (name: f name attrs.${name}) (sortedAttrNames attrs);

  realmRows = cfg._index.realms.enabledList;
  enabledVms = cfg._index.enabledVms;
  runtimeRows = cfg._index.runtime.byVm;
  runtimeProviders = cfg._index.runtime.providers;
  allocatorData = cfg._bundle.allocatorJson.data;
  allocatorConfigPath = "/etc/d2b/allocator.json";

  providerPlacementFor = realm:
    if realm.placementProvider == null then null
    else
      let
        provider =
          if builtins.hasAttr realm.placementProvider realm.providers
          then realm.providers.${realm.placementProvider}
          else null;
      in
      {
        providerName = realm.placementProvider;
        providerId =
          if provider != null
          then provider.id
          else realm.placementProvider;
        kind =
          if provider != null
          then provider.kind
          else null;
        providerSpecificPlacement = realm.providerSpecificPlacement;
      };

  providerConfig = providerName: provider: {
    inherit providerName;
    providerId = provider.id;
    enabled = provider.enabled;
    kind = provider.kind;
    placement = provider.placement;
    capabilityRefs = sortNames (lib.unique provider.capabilityRefs);
    configRef = provider.configRef;
  };

  resourceRefsFor = realm:
    sortNames (map (request: request.resourceId)
      (lib.filter (request: request.realmPath == realm.path)
        allocatorData.resourceRequests));

  localRuntimeWorkloadsFor = realm:
    if realm.placement != "host-local" || realm.network.env == null then [ ]
    else
      sortedMapAttrsToList
        (vmName: vm:
          if vm.env == realm.network.env then
            let
              runtime = runtimeRows.${vmName}.metadata;
            in
            {
              workloadId = vmName;
              inherit vmName;
              env = vm.env;
              inherit runtime;
              paths = {
                stateDir = cfg.manifest.${vmName}.stateDir;
                runDir = "/run/d2b/vms/${vmName}";
                storeView = "${toString cfg.store.stateDir}/${vmName}/store-view";
                guestControlDir = "/run/d2b/vms/${vmName}/guest-control";
              };
            }
          else null)
        enabledVms;

  compact = values: lib.filter (value: value != null) values;

  runtimeProviderById = providerId:
    lib.findFirst
      (provider: provider.provider.id == providerId)
      (throw "d2b realm-controller-config: local runtime provider '${providerId}' is missing from runtime provider catalog")
      runtimeProviders;

  localRuntimeFor = realm:
    let
      workloads = compact (localRuntimeWorkloadsFor realm);
      providerIds = sortNames (lib.unique (map (workload: workload.runtime.provider.id) workloads));
    in
    if workloads == [ ] then null
    else {
      runtimeState = "metadata-only";
      providers = map runtimeProviderById providerIds;
      inherit workloads;
      invariants = {
        metadataOnly = true;
        existingGlobalVmPathsPreserved = true;
        noStateMigrationDuringActivation = true;
        brokerEffectsRemainRealmDelegated = true;
      };
    };

  controllerConfig = realm:
    let
      controller = realm.controller;
    in
    {
      realmName = realm.realmName;
      realmId = realm.id;
      realmPath = realm.path;
      placement = realm.placement;
      providerPlacement = providerPlacementFor realm;
      daemon = controller.daemon;
      broker = controller.broker;
      paths = {
        runDir = realm.paths.runDir;
        stateDir = realm.paths.stateDir;
        auditDir = realm.paths.auditDir;
      };
      sockets = {
        publicSocketPath = realm.paths.publicSocket;
        brokerSocketPath = realm.paths.brokerSocket;
      };
      allocator = {
        kind = "local-root-metadata";
        configPath = allocatorConfigPath;
        rootSocket = allocatorData.allocator.rootSocket;
        resourceRequestRefs = resourceRefsFor realm;
      };
      access = {
        allowedUsers = realm.allowedUsers;
        allowedGroups = realm.allowedGroups;
        inheritedAdminUsers = sortNames (lib.unique cfg.site.adminUsers);
      };
      localRuntime = localRuntimeFor realm;
      providers = sortedMapAttrsToList providerConfig realm.providers;
    };

  data = {
    schemaVersion = "v2";
    runtimeState = "metadata-only";
    controllers = map controllerConfig realmRows;
    invariants = {
      metadataOnly = true;
      noSystemdUnitsMaterialized = !lib.any
        (realm:
          realm.controller.daemon.materializedService
          || realm.controller.broker.materializedSocket
          || realm.controller.broker.materializedService)
        realmRows;
      preservesGlobalDaemonBehavior = true;
      preservesDirectUnixSocketSemantics = true;
    };
  };
in
{
  config = {
    assertions = [
      {
        assertion =
          lib.all
            (realm:
              builtins.substring 0 9 realm.controller.daemon.stateLockPath == "/run/d2b/"
              && builtins.substring 0 9 realm.controller.daemon.locksDir == "/run/d2b/")
            realmRows;
        message = "realm controller daemon lock metadata must remain under /run/d2b while runtime is metadata-only.";
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
