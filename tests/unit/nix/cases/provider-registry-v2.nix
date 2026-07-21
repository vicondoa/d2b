{ flakeRoot, lib, ... }:

let
  schema = builtins.fromJSON
    (builtins.readFile
      "${flakeRoot}/docs/reference/schemas/v2/provider-registry-v2.json");
  identity = import
    (flakeRoot + "/nixos-modules/v2-identity.nix");
  bindingVariants = schema.definitions.ProviderBindingV2.oneOf;
  bindingAxes = map
    (variant: builtins.head variant.properties.axis.enum)
    bindingVariants;
  bindingByAxis = builtins.listToAttrs (map
    (variant: {
      name = builtins.head variant.properties.axis.enum;
      value = variant;
    })
    bindingVariants);
  compose = index: (lib.evalModules {
      specialArgs.pkgs = { };
      modules = [
        (flakeRoot + "/nixos-modules/provider-registry-v2-json.nix")
        ({ lib, ... }: {
          options.d2b = lib.mkOption {
            type = lib.types.attrs;
          };
          config.d2b = {
            realms = { };
            observability = {
              enable = index.observabilityEnabled or false;
              vmName = "sys-obs";
            };
            _index = index;
          };
        })
      ];
    }).config.d2b._bundle.providerRegistryV2Json.data.providers;
  emptyIndex = {
    realms = {
      enabledList = [ ];
      enabledByPath = { };
      byId = { };
    };
    workloads.enabledList = [ ];
    roles = {
      list = [ ];
      byWorkloadId = { };
    };
    providers = {
      enabledList = [ ];
      byId = { };
      bindingsByWorkloadId = { };
    };
    devices.list = [ ];
    resources = {
      list = [ ];
      byId = { };
      byWorkloadId = { };
    };
    providerRegistryV2Mappings = {
      transport = [ ];
      substrate = [ ];
      display = [ ];
    };
  };
  composedEmptyRegistry = compose emptyIndex;
  runtimeRealmId = identity.deriveRealmId "work.local-root";
  runtimeWorkloadId = identity.deriveWorkloadId runtimeRealmId "desktop";
  configuredRuntimeProviderId =
    identity.deriveProviderId runtimeRealmId "runtime" "primary";
  runtimeRoleId =
    identity.deriveRoleId runtimeRealmId runtimeWorkloadId "cloud-hypervisor";
  runtimeIndex = emptyIndex // {
    realms = emptyIndex.realms // {
      byId = {
        ${runtimeRealmId} = {
          realmId = runtimeRealmId;
          realmPath = "work.local-root";
          placement = "host-local";
        };
      };
    };
    workloads.enabledList = [
      {
        enabled = true;
        realmId = runtimeRealmId;
        realmPath = "work.local-root";
        workloadId = runtimeWorkloadId;
        workloadName = "desktop";
        canonicalTarget = "desktop.work.local-root.d2b";
        providerBindings.runtime = {
          implementationId = "cloud-hypervisor";
          providerId = configuredRuntimeProviderId;
          providerType = "runtime";
        };
        spec = { };
      }
    ];
    roles = emptyIndex.roles // {
      byWorkloadId = {
        ${runtimeWorkloadId} = [
          {
            realmId = runtimeRealmId;
            workloadId = runtimeWorkloadId;
            roleId = runtimeRoleId;
            roleKind = "cloud-hypervisor";
          }
        ];
      };
    };
    providers = emptyIndex.providers // {
      byId = {
        ${configuredRuntimeProviderId} = {
          enabled = true;
          realmId = runtimeRealmId;
          providerId = configuredRuntimeProviderId;
          providerType = "runtime";
          implementationId = "cloud-hypervisor";
          placement = "host-local";
        };
      };
      bindingsByWorkloadId = {
        ${runtimeWorkloadId}.runtime = {
          implementationId = "cloud-hypervisor";
          providerId = configuredRuntimeProviderId;
          providerType = "runtime";
        };
      };
    };
  };
  composedRuntimeEntry = builtins.head (compose runtimeIndex);
  expectedRuntimeProviderId = identity.deriveProviderId
    runtimeRealmId "runtime" "runtime-${runtimeWorkloadId}";
  disabledRuntimeProviderIndex = lib.recursiveUpdate runtimeIndex {
    providers.byId.${configuredRuntimeProviderId}.enabled = false;
  };
  localRootRealmId = identity.deriveRealmId "local-root";
  observabilityWorkloadId =
    identity.deriveWorkloadId localRootRealmId "sys-obs";
  configuredObservabilityProviderId =
    identity.deriveProviderId
      localRootRealmId "observability" "observability-local";
  observabilityIndex = emptyIndex // {
    observabilityEnabled = true;
    realms = emptyIndex.realms // {
      enabledByPath.local-root = {
        realmId = localRootRealmId;
        realmPath = "local-root";
        placement = "host-local";
      };
    };
    workloads = emptyIndex.workloads // {
      byId.${observabilityWorkloadId} = {
        enabled = true;
        configuredName = "sys-obs";
        realmId = localRootRealmId;
        realmPath = "local-root";
        workloadId = observabilityWorkloadId;
        providerBindings.observability = {
          implementationId = "local";
          providerId = configuredObservabilityProviderId;
          providerType = "observability";
        };
      };
    };
    providers = emptyIndex.providers // {
      byId.${configuredObservabilityProviderId} = {
        enabled = true;
        realmId = localRootRealmId;
        providerId = configuredObservabilityProviderId;
        providerType = "observability";
        implementationId = "local";
        placement = "host-local";
      };
    };
  };
  composedObservabilityEntry =
    builtins.head (compose observabilityIndex);
  missingObservabilityBindingIndex = observabilityIndex // {
    workloads = observabilityIndex.workloads // {
      byId = observabilityIndex.workloads.byId // {
        ${observabilityWorkloadId} =
          observabilityIndex.workloads.byId.${observabilityWorkloadId} // {
            providerBindings = { };
          };
      };
    };
  };
in
{
  "provider-registry-v2/closed-binding-axis-set" = {
    expr = bindingAxes;
    expected = [
      "local-runtime"
      "local-observability"
      "local-transport"
      "local-substrate"
      "local-display"
      "network"
      "local-storage"
      "local-device"
      "local-audio"
    ];
  };

  "provider-registry-v2/every-binding-variant-is-closed" = {
    expr = lib.all
      (variant: variant.additionalProperties == false)
      bindingVariants;
    expected = true;
  };

  "provider-registry-v2/integrated-fragment-fields-are-frozen" = {
    expr = {
      storageRequired =
        lib.sort lib.lessThan bindingByAxis.local-storage.required;
      audioRequired =
        lib.sort lib.lessThan bindingByAxis.local-audio.required;
      deviceRequired =
        lib.sort lib.lessThan bindingByAxis.local-device.required;
      deviceResourceMax =
        bindingByAxis.local-device.properties.deviceResourceIds.maxItems;
    };
    expected = {
      storageRequired = [
        "axis"
        "closureSyncId"
        "diskSetId"
        "localStateId"
        "mediaSetId"
        "realmId"
        "resourceGeneration"
        "storeViewId"
        "workloadId"
      ];
      audioRequired = [
        "axis"
        "endpointId"
        "leaseId"
        "lockStorageId"
        "mediationStorageId"
        "processId"
        "roleId"
        "stateStorageId"
        "workloadId"
      ];
      deviceRequired = [
        "axis"
        "deviceResourceIds"
      ];
      deviceResourceMax = 64;
    };
  };

  "provider-registry-v2/loads-direct-fragments-into-one-artifact" = {
    expr = composedEmptyRegistry;
    expected = [ ];
  };

  "provider-registry-v2/projects-authoritative-runtime-process-intents" = {
    expr = {
      providerId = composedRuntimeEntry.descriptor.providerId;
      configuredProviderIdIsNotDescriptorId =
        configuredRuntimeProviderId != composedRuntimeEntry.descriptor.providerId;
      realmId = composedRuntimeEntry.descriptor.placement.realmId;
      controllerRole =
        composedRuntimeEntry.descriptor.placement.controllerRole;
      implementationId = composedRuntimeEntry.descriptor.implementationId;
      binding = composedRuntimeEntry.binding;
    };
    expected = {
      providerId = expectedRuntimeProviderId;
      configuredProviderIdIsNotDescriptorId = true;
      realmId = runtimeRealmId;
      controllerRole = "realm-controller";
      implementationId = "cloud-hypervisor";
      binding = {
        axis = "local-runtime";
        workloadId = runtimeWorkloadId;
        vmStartIntentId =
          "vm-start:workload:${runtimeWorkloadId}:role:${runtimeRoleId}";
        runnerIntentId =
          "runner:workload:${runtimeWorkloadId}:role:${runtimeRoleId}";
      };
    };
  };

  "provider-registry-v2/rejects-disabled-normalized-runtime-authority" = {
    expr = !(builtins.tryEval
      (builtins.deepSeq (compose disabledRuntimeProviderIndex) true)).success;
    expected = true;
  };

  "provider-registry-v2/uses-canonical-local-root-observability-identity" = {
    expr = {
      providerId = composedObservabilityEntry.descriptor.providerId;
      realmId = composedObservabilityEntry.descriptor.placement.realmId;
      controllerRole =
        composedObservabilityEntry.descriptor.placement.controllerRole;
      binding = composedObservabilityEntry.binding;
    };
    expected = {
      providerId = identity.deriveProviderId
        localRootRealmId "observability" "observability-local";
      realmId = localRootRealmId;
      controllerRole = "local-root-controller";
      binding = {
        axis = "local-observability";
        maxRecords = 64;
        maxBytes = 32768;
        maxTimeWindowMs = 86400000;
      };
    };
  };

  "provider-registry-v2/rejects-missing-normalized-observability-authority" = {
    expr = !(builtins.tryEval
      (builtins.deepSeq
        (compose missingObservabilityBindingIndex)
        true)).success;
    expected = true;
  };

}
