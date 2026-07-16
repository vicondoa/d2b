{ config, lib, pkgs, ... }:

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

  mappedRuntimeRows = import ./workload-process-rows.nix {
    inherit config lib pkgs;
  };

  runtimeEntry = row:
    let
      canonicalRealmId = row.realmId;
      canonicalWorkloadId = row.workloadId;
      runtimeBinding =
        if row.runtimeBinding == null
        then throw "provider registry local-runtime mapping has no normalized runtime provider"
        else row.runtimeBinding;
      runtimeProvider =
        cfg._index.providers.byId.${runtimeBinding.providerId}
          or (throw "provider registry local-runtime mapping references an unknown provider");
      realm =
        cfg._index.realms.byId.${canonicalRealmId}
          or (throw "provider registry local-runtime mapping references an unknown realm");
      configuredProviderId = "runtime-${canonicalWorkloadId}";
      canonicalProviderId = identity.deriveProviderId
        canonicalRealmId "runtime" configuredProviderId;
      implementationId = row.runtimeImplementation;
      vmStartIntentId = row.vmStartIntentId;
      runnerIntentId = row.runnerIntentId;
      normalizedAuthorityMatches =
        runtimeBinding.providerType == "runtime"
        && runtimeBinding.implementationId == implementationId
        && runtimeProvider.enabled
        && runtimeProvider.providerType == "runtime"
        && runtimeProvider.realmId == canonicalRealmId
        && runtimeProvider.implementationId == implementationId
        && runtimeProvider.placement == "host-local"
        && builtins.elem implementationId [ "cloud-hypervisor" "qemu-media" ];
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
      controllerRole =
        if realm.realmPath == "local-root"
        then "local-root-controller"
        else "realm-controller";
    in
    if !normalizedAuthorityMatches
    then throw "provider registry local-runtime mapping disagrees with normalized authority"
    else {
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
          inherit controllerRole;
        };
      };
      binding = {
        axis = "local-runtime";
        inherit (row) workloadId vmStartIntentId runnerIntentId;
      };
    };

  localRootRealm =
    cfg._index.realms.enabledByPath."local-root" or null;
  mappedObservabilityRealms = lib.optional
    (localRootRealm != null && localRootRealm.placement == "host-local")
    localRootRealm;

  observabilityEntry = realm:
    let
      canonicalRealmId = realm.realmId;
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

  extensionFragmentPaths = [
    ./provider-registry-v2-extensions/transport.nix
    ./provider-registry-v2-extensions/substrate.nix
    ./provider-registry-v2-extensions/display.nix
    ./provider-registry-v2-extensions/network.nix
    ./provider-registry-v2-extensions/storage.nix
    ./provider-registry-v2-extensions/device.nix
    ./provider-registry-v2-extensions/audio.nix
  ];
  fragmentContext = {
    inherit config cfg generation identity lib pkgs;
  };
  loadFragment = path:
    let
      fragment = lib.callPackageWith fragmentContext path { };
    in
    if builtins.isAttrs fragment
      && fragment ? providers
      && builtins.isList fragment.providers
    then fragment.providers
    else throw "provider-registry-v2 fragment must return { providers = [ ... ]; }";
  extensionProviders =
    lib.concatMap loadFragment extensionFragmentPaths;

  providers = lib.sort
    (left: right:
      lib.lessThan left.descriptor.providerId right.descriptor.providerId)
    ((map runtimeEntry mappedRuntimeRows)
      ++ (map observabilityEntry mappedObservabilityRealms)
      ++ extensionProviders);
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
