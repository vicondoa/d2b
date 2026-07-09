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

  # Build a local runtime workload entry for a given vmName.
  # workloadRow is the index entry for an explicit workload declaration, or
  # null for transitional env-based workload entries that have no
  # corresponding realm workload declaration yet.
  mkLocalWorkloadEntry = workloadId: vmName: workloadRow:
    let
      vm = enabledVms.${vmName};
      runtime = runtimeRows.${vmName}.metadata;
      base = {
        inherit workloadId vmName;
        env = if vm.env != null then vm.env else "none";
        inherit runtime;
        paths = {
          stateDir = cfg.manifest.${vmName}.stateDir;
          runDir = "/run/d2b/vms/${vmName}";
          storeView = "${toString cfg.store.stateDir}/${vmName}/store-view";
          guestControlDir = "/run/d2b/vms/${vmName}/guest-control";
        };
      };
      # Realm-native workload identity: present for explicit realm workload
      # entries, absent for transitional env-based entries (no declaration to
      # source from).  Field names and nesting must match WorkloadIdentity in
      # d2b-core (deny_unknown_fields; required: workloadId, realmId,
      # realmPath, canonicalTarget; optional: legacyVmName, runtimeKind,
      # providerId).
      identity =
        if workloadRow != null then
          lib.filterAttrs (_: v: v != null) {
            inherit workloadId;
            realmId = workloadRow.realmId;
            realmPath = lib.splitString "." workloadRow.realmPath;
            canonicalTarget = workloadRow.canonicalTarget;
            legacyVmName = workloadRow.legacyVmName;
            runtimeKind = workloadRow.runtimeKind;
            providerId = workloadRow.runtimeProviderId;
          }
        else null;
    in base // lib.optionalAttrs (identity != null) { inherit identity; };

  localRuntimeWorkloadsFor = realm:
    let
      # Explicit workload rows from realm.workloads that reference an enabled VM.
      explicitRows = lib.filter
        (row:
          row.enable
          && row.realmName == realm.realmName
          && row.legacyVmName != null
          && builtins.hasAttr row.legacyVmName enabledVms)
        cfg._index.realms.workloads.enabled;
      explicitVmNames = map (row: row.legacyVmName) explicitRows;

      explicitEntries = map
        (row: mkLocalWorkloadEntry row.workloadName row.legacyVmName row)
        explicitRows;

      # Transitional env-based workloads: VMs in realm.network.env that are
      # not already covered by an explicit workload declaration. Preserved for
      # backward compat when realm.workloads is empty or does not cover all
      # env-member VMs.
      envBasedEntries =
        if realm.placement != "host-local" || realm.network.env == null
        then [ ]
        else
          lib.filter (entry: entry != null)
            (sortedMapAttrsToList
              (vmName: vm:
                if vm.env == realm.network.env && !(builtins.elem vmName explicitVmNames)
                then mkLocalWorkloadEntry vmName vmName null
                else null)
              enabledVms);
    in
    lib.sortOn (w: w.workloadId) (explicitEntries ++ envBasedEntries);

  compact = values: lib.filter (value: value != null) values;

  runtimeProviderById = providerId:
    lib.findFirst
      (provider: provider.provider.id == providerId)
      (throw "d2b realm-controller-config: local runtime provider '${providerId}' is missing from runtime provider catalog")
      runtimeProviders;

  localRuntimeFor = realm:
    let
      workloads = localRuntimeWorkloadsFor realm;
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
