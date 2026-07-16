{ config, lib, ... }:

let
  cfg = config.d2b;
  identity = import ./v2-identity.nix;
  generation = 1;
  runtimeConfigurationSchemaFingerprint =
    builtins.hashString "sha256" "d2b-provider-runtime-local-configuration-v1";
  observabilityConfigurationSchemaFingerprint =
    builtins.hashString "sha256" "d2b-provider-observability-local-configuration-v1";
  observabilityLimits = {
    maxRecords = 64;
    maxBytes = 32768;
    maxTimeWindowMs = 86400000;
  };
  liveRuntimeCapabilities = [
    "runtime.plan"
    "runtime.ensure"
    "runtime.start"
    "runtime.stop"
    "runtime.inspect"
    "runtime.adopt"
    "runtime.destroy"
  ];
  liveObservabilityCapabilities = [
    "observability.status"
    "observability.query"
    "observability.export"
  ];

  mappedRuntimeRows = lib.filter
    (row:
      row.enable
      && builtins.elem row.kind [ "local-vm" "qemu-media" ]
      && row.legacyVmName != null
      && builtins.hasAttr row.legacyVmName cfg._index.enabledVms
      && (cfg._index.runtime.byVm.${row.legacyVmName}.kind
        == (if row.kind == "qemu-media" then "qemu-media" else "nixos")))
    cfg._index.realms.workloads.enabled;

  runtimeEntry = row:
    let
      canonicalRealmId = identity.deriveRealmId "${row.realmPath}.local-root";
      canonicalWorkloadId =
        identity.deriveWorkloadId canonicalRealmId row.workloadName;
      configuredProviderId = "runtime-${canonicalWorkloadId}";
      canonicalProviderId = identity.deriveProviderId
        canonicalRealmId "runtime" configuredProviderId;
      implementationId =
        if row.kind == "qemu-media" then "qemu-media" else "cloud-hypervisor";
      roleId = implementationId;
      vmStartIntentId =
        "vm-start:vm:${row.legacyVmName}:role:${roleId}";
      runnerIntentId =
        "runner:vm:${row.legacyVmName}:role:${roleId}";
      scopeDigest = builtins.hashString "sha256" (builtins.toJSON {
        providerId = canonicalProviderId;
        realmId = canonicalRealmId;
        workloadId = canonicalWorkloadId;
        inherit vmStartIntentId runnerIntentId;
      });
      posture =
        if implementationId == "cloud-hypervisor" then {
          process = "provider-owned-pidfd";
          cgroup = "realm-delegated-leaf";
          network = "isolated-namespace";
          userNamespace = "broker-preestablished";
          persistentIdentity = "file-backed-cloneable";
          deviceMediation = "broker-delegated-typed";
        } else {
          process = "provider-owned-pidfd";
          cgroup = "realm-delegated-leaf";
          network = "isolated-namespace";
          userNamespace = "none";
          persistentIdentity = "none";
          deviceMediation = "broker-delegated-typed";
        };
    in {
      descriptor = {
        schemaVersion = 2;
        providerId = canonicalProviderId;
        authority = {
          type = "runtime";
          inherit posture;
        };
        inherit implementationId;
        apiVersion = {
          major = 2;
          minor = 0;
        };
        capabilities = liveRuntimeCapabilities;
        configurationSchemaFingerprint = runtimeConfigurationSchemaFingerprint;
        configuredScopeDigest = scopeDigest;
        registryGeneration = generation;
        placement = {
          kind = "trusted-first-party-in-process";
          realmId = canonicalRealmId;
          controllerRole = "realm-controller";
        };
      };
      binding = {
        axis = "local-runtime";
        workloadId = canonicalWorkloadId;
        inherit vmStartIntentId runnerIntentId;
      };
    };

  mappedObservabilityRealms = lib.filter
    (realm:
      realm.placement == "host-local"
      && realm.parentPath == null)
    cfg._index.realms.enabledList;

  observabilityEntry = realm:
    let
      canonicalRealmId = identity.deriveRealmId "${realm.path}.local-root";
      canonicalProviderId = identity.deriveProviderId
        canonicalRealmId "observability" "observability-local";
      scopeDigest = builtins.hashString "sha256" (builtins.toJSON ({
        providerId = canonicalProviderId;
      } // observabilityLimits));
    in {
      descriptor = {
        schemaVersion = 2;
        providerId = canonicalProviderId;
        authority = {
          type = "observability";
        };
        implementationId = "local";
        apiVersion = {
          major = 2;
          minor = 0;
        };
        capabilities = liveObservabilityCapabilities;
        configurationSchemaFingerprint =
          observabilityConfigurationSchemaFingerprint;
        configuredScopeDigest = scopeDigest;
        registryGeneration = generation;
        placement = {
          kind = "trusted-first-party-in-process";
          realmId = canonicalRealmId;
          controllerRole = "local-root-controller";
        };
      };
      binding = {
        axis = "local-observability";
      } // observabilityLimits;
    };

  providers = lib.sort
    (left: right:
      lib.lessThan left.descriptor.providerId right.descriptor.providerId)
    ((map runtimeEntry mappedRuntimeRows)
      ++ (map observabilityEntry mappedObservabilityRealms));
  configurationFingerprint = builtins.hashString "sha256" (builtins.toJSON {
    schemaVersion = "v2";
    registryGeneration = generation;
    inherit providers;
  });
in
{
  config.d2b._bundle.providerRegistryV2Json = {
    data = {
      schemaVersion = "v2";
      registryGeneration = generation;
      inherit configurationFingerprint;
      publishedAtUnixMs = 0;
      inherit providers;
    };
    installFileName = "provider-registry-v2.json";
    classification = "contractPrivateNonSecret";
    sensitivity = "nonSecret";
  };
}
