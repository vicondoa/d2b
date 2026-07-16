# Canonical public presentation metadata for desktop clients.
{ config, lib, ... }:

let
  cfg = config.d2b;

  realms = cfg._index.realms.enabledList;
  workloads = cfg._index.workloads.enabledList;
  providers = cfg._index.providers.enabledList;

  realmById = lib.listToAttrs (map
    (realm: {
      name = realm.realmId;
      value = realm;
    })
    realms);

  publicIcon = icon:
    lib.filterAttrs (_: value: value != null) icon;
  publicItem = itemId: item: {
    id = itemId;
    inherit (item) type name graphical;
    icon = publicIcon item.icon;
    capabilities =
      if item.type == "shell"
      then [ "persistent-shell" "pty" ]
      else [ "configured-launch" ]
        ++ lib.optional item.graphical "window-forwarding";
  };

  realmEntry = realm: {
    name = realm.realmId;
    value = {
      inherit (realm) realmId;
      canonicalTarget = realm.canonicalTargetSuffix;
      label = realm.metadata.name;
      accentColor = cfg._uiColors.realms.${realm.realmName}.accent;
    };
  };

  executionPosture = implementationId:
    if implementationId == "systemd-user"
    then {
      isolation = "unsafe-local";
      environment = "systemd-user-manager-ambient";
      displayEnvironment = "wayland-proxy-only";
      executionIdentity = "authenticated-requester-uid";
      sessionPersistence = "user-manager-lifetime";
    }
    else if builtins.elem implementationId [ "cloud-hypervisor" "qemu-media" ]
    then {
      isolation = "virtual-machine";
      environment = "runtime-managed";
      displayEnvironment = "runtime-managed";
      executionIdentity = "workload-user";
      sessionPersistence = "runtime-managed";
    }
    else {
      isolation = "provider-managed";
      environment = "runtime-managed";
      displayEnvironment = "runtime-managed";
      executionIdentity = "provider-managed";
      sessionPersistence = "runtime-managed";
    };

  workloadEntry = workload:
    let
      realm = realmById.${workload.realmId};
      runtimeProvider = workload.providerBindings.runtime;
    in
    {
      name = workload.canonicalTarget;
      value = {
        inherit (workload) canonicalTarget realmId workloadId;
        providerId = runtimeProvider.providerId;
        executionPosture = executionPosture runtimeProvider.implementationId;
        label = workload.metadata.label;
        icon = publicIcon workload.metadata.icon;
        realmAccentColor = cfg._uiColors.realms.${workload.realmName}.accent;
        launcherEnabled = workload.launcher.enabled;
        defaultItemId = workload.launcher.defaultItem;
        capabilities = workload.capabilityRefs;
        items = lib.mapAttrsToList publicItem workload.launcher.items;
      };
    };

  providerEntry = provider:
    let
      realm = realmById.${provider.realmId};
    in
    {
      name = provider.providerId;
      value = {
        inherit (provider) providerId realmId;
        canonicalTarget =
          "${provider.configuredProviderId}.${realm.canonicalTargetSuffix}";
        implementation = provider.implementationId;
        label = provider.metadata.label;
        capabilities = provider.capabilityRefs;
      };
    };

  realmEntries = map realmEntry realms;
  workloadEntries = map workloadEntry workloads;
  providerEntries = map providerEntry providers;

  uniqueNames = entries:
    lib.length entries
    == lib.length (lib.unique (map (entry: entry.name) entries));

  data = {
    schemaVersion = "v2";
    runtimeState = "presentation-only";
    realms = lib.listToAttrs realmEntries;
    workloads = lib.listToAttrs workloadEntries;
    providers = lib.listToAttrs providerEntries;
    limits = {
      maxRealms = 64;
      maxWorkloads = 256;
      maxProviders = 256;
      maxItemsPerWorkload = 64;
      maxCapabilitiesPerEntry = 64;
    };
    invariants = {
      argvPrivate = true;
      canonicalIdsOnly = true;
      canonicalTargetsOnly = true;
      colorsArePresentationOnly = true;
      metadataIsNotAuthorization = true;
      nonAuthoritativeProjection = true;
      noSecretsOrCredentials = true;
    };
  };

  countsWithinBounds =
    lib.length realmEntries <= data.limits.maxRealms
    && lib.length workloadEntries <= data.limits.maxWorkloads
    && lib.length providerEntries <= data.limits.maxProviders
    && lib.all
      (workload:
        lib.length (lib.attrNames workload.launcher.items)
          <= data.limits.maxItemsPerWorkload
        && lib.length workload.capabilityRefs
          <= data.limits.maxCapabilitiesPerEntry
        && lib.all
          (item:
            lib.length (publicItem "bounded-item" item).capabilities
            <= data.limits.maxCapabilitiesPerEntry)
          (builtins.attrValues workload.launcher.items))
      workloads
    && lib.all
      (provider:
        lib.length provider.capabilityRefs
          <= data.limits.maxCapabilitiesPerEntry)
      providers;
in
{
  config = {
    assertions = [
      {
        assertion =
          uniqueNames realmEntries
          && uniqueNames workloadEntries
          && uniqueNames providerEntries;
        message = "d2b desktop metadata canonical ids and targets must be unique";
      }
      {
        assertion = countsWithinBounds;
        message = "d2b desktop metadata exceeds its public projection limits";
      }
    ];

    d2b._bundle.extraArtifacts.desktopMetadataJson = {
      inherit data;
      installFileName = "desktop-metadata.json";
      classification = "contractPublic";
      sensitivity = "nonSecret";
    };
  };
}
