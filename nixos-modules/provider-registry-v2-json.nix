{ config, lib, ... }:

let
  cfg = config.d2b;
  identity = import ./v2-identity.nix;
  generation = 1;
  configurationSchemaFingerprint =
    builtins.hashString "sha256" "d2b-provider-runtime-local-configuration-v1";
  liveRuntimeCapabilities = [
    "runtime.plan"
    "runtime.ensure"
    "runtime.start"
    "runtime.stop"
    "runtime.inspect"
    "runtime.adopt"
    "runtime.destroy"
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
        inherit configurationSchemaFingerprint;
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

  providers = lib.sort
    (left: right:
      lib.lessThan left.descriptor.providerId right.descriptor.providerId)
    (map runtimeEntry mappedRuntimeRows);
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
