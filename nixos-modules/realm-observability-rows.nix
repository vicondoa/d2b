{ config, lib }:

let
  cfg = config.d2b;
  identity = import ./v2-identity.nix;
  realmId = identity.deriveRealmId "local-root";
  workloadId = identity.deriveWorkloadId realmId cfg.observability.vmName;
  runtimeProviderId =
    identity.deriveProviderId realmId "runtime" "runtime-local";
  bridgeRoleId =
    identity.deriveRoleId realmId workloadId "vsock-relay";

  stateRoot = "/var/lib/d2b/r/${realmId}/w/${workloadId}";
  runRoot = "/run/d2b/r/${realmId}/w/${workloadId}";
  configRoot = "/etc/d2b/r/${realmId}/w/${workloadId}";
  auditRoot = "/var/lib/d2b/r/${realmId}/audit";

  enabledSources = lib.filter
    (workload:
      workload.enabled
      && workload.workloadId != workloadId)
    cfg._index.workloads.enabledList;
  sortedSources = lib.sort
    (left: right:
      lib.lessThan left.canonicalTarget right.canonicalTarget)
    enabledSources;
  sourceRows = lib.imap0
    (index: workload: {
      sourceId = workload.workloadId;
      sourceName = workload.configuredName;
      inherit (workload) canonicalTarget realmId workloadId;
      realmPath = workload.realmPath;
      role = "workload";
      vsockPort = 14318 + index;
      receiverGrpcPort = 14318 + index;
      receiverHttpPort = null;
      projection = {
        policy = "positive-allowlist";
        fields = [
          "kind"
          "operation"
          "outcome"
          "provider"
          "realmId"
          "timestamp"
          "workloadId"
        ];
        excluded = [
          "argv"
          "commandOutput"
          "credentials"
          "environment"
          "hostPath"
          "rawAudit"
          "secret"
        ];
      };
    })
    sortedSources;
  ingressSources = lib.listToAttrs (
    [
      {
        name = "host";
        value = {
          vmName = cfg.observability.host.identityName;
          envName = "local-root";
          role = "host";
          vsockPort = 14317;
          receiverGrpcPort = cfg.observability.signoz.otlpGrpcPort;
          receiverHttpPort = cfg.observability.signoz.otlpHttpPort;
        };
      }
    ]
    ++ map
      (source: {
        name = source.sourceName;
        value = {
          vmName = source.sourceName;
          envName = source.realmPath;
          inherit (source) role vsockPort receiverGrpcPort receiverHttpPort;
        };
      })
      sourceRows
  );

  pathRow = id: kind: path: sensitivity: {
    inherit id kind path realmId workloadId sensitivity;
    scope = "workload:${workloadId}";
    creator = "realm-broker";
    repairOwner = "realm-broker";
    noFollow = true;
  };

  registryProviders =
    lib.attrByPath
      [ "_bundle" "providerRegistryV2Json" "data" "providers" ]
      [ ]
      cfg;
  localObservabilityEntries = lib.filter
    (entry: (entry.binding.axis or null) == "local-observability")
    registryProviders;
  localObservabilityEntry =
    if builtins.length localObservabilityEntries == 1
    then builtins.head localObservabilityEntries
    else throw
      "realm observability requires exactly one frozen local-observability registry mapping";
  frozenBinding = localObservabilityEntry.binding;
in
{
  schemaVersion = 1;
  enabled = cfg.observability.enable;

  workload = {
    inherit realmId workloadId runtimeProviderId;
    configuredName = cfg.observability.vmName;
    canonicalTarget = "${cfg.observability.vmName}.local-root.d2b";
    autostart = true;
  };

  roles = [
    {
      roleId = bridgeRoleId;
      roleKind = "vsock-relay";
      inherit realmId workloadId;
      purpose = "otel-host-bridge";
    }
  ];

  endpoints = {
    hostEgress = {
      id = "endpoint:observability-host-egress:${workloadId}";
      kind = "unix-stream";
      path = "${runRoot}/sockets/host-egress.sock";
      inherit realmId workloadId bridgeRoleId;
      mode = "0660";
    };
    hostIngest = {
      id = "endpoint:observability-host-ingest:${workloadId}";
      kind = "unix-stream";
      path = "${runRoot}/sockets/ingest/host-otlp.sock";
      inherit realmId workloadId;
      mode =
        if cfg.observability.host.otlpIngest.clientGroup == null
        then "0600"
        else "0660";
    };
    stackVsock = {
      id = "endpoint:observability-stack-vsock:${workloadId}";
      kind = "cloud-hypervisor-vsock";
      path = "${stateRoot}/vsock.sock";
      port = 14317;
      inherit realmId workloadId;
    };
  };

  paths = [
    (pathRow
      "path:observability-config:${workloadId}"
      "config"
      "${configRoot}/observability"
      "contract-private")
    (pathRow
      "path:observability-state:${workloadId}"
      "state"
      "${stateRoot}/observability"
      "secret-adjacent")
    (pathRow
      "path:observability-secrets:${workloadId}"
      "secret-source"
      "${stateRoot}/observability/secrets"
      "secret")
    (pathRow
      "path:observability-runtime:${workloadId}"
      "runtime"
      "${runRoot}/sockets"
      "realm-scoped")
    (pathRow
      "path:observability-store-sync-projection:${workloadId}"
      "bounded-projection"
      "${auditRoot}/projections/store-sync"
      "contract-private")
  ];

  secrets = [
    {
      id = "secret:observability-signoz-jwt:${workloadId}";
      fileName = "signoz-jwt-secret";
      source = cfg.observability.signoz.jwtSecretFile;
      generatedBytes = 64;
      minimumBytes = 32;
      mode = "0400";
      inherit realmId workloadId;
      path = "${stateRoot}/observability/secrets/signoz-jwt-secret";
      owner = "realm-broker";
    }
    {
      id = "secret:observability-signoz-root-password:${workloadId}";
      fileName = "signoz-root-password";
      source = cfg.observability.signoz.rootPasswordFile;
      generatedBytes = 48;
      minimumBytes = 16;
      mode = "0400";
      inherit realmId workloadId;
      path = "${stateRoot}/observability/secrets/signoz-root-password";
      owner = "realm-broker";
    }
    {
      id = "secret:observability-clickhouse-password:${workloadId}";
      fileName = "clickhouse-password";
      source = cfg.observability.signoz.clickhousePasswordFile;
      generatedBytes = 48;
      minimumBytes = 16;
      mode = "0400";
      inherit realmId workloadId;
      path = "${stateRoot}/observability/secrets/clickhouse-password";
      owner = "realm-broker";
    }
  ];

  projections = {
    provider = {
      providerId = localObservabilityEntry.descriptor.providerId;
      axis = frozenBinding.axis;
      registration = "frozen-provider-registry-v2";
      limits = builtins.removeAttrs frozenBinding [ "axis" ];
    };
    policy = {
      bounded = true;
      rawAuditAccess = false;
      rawRepairStateAccess = false;
      export = "durable-atomic-rename";
      redaction = "positive-allowlist";
      inherit sourceRows;
    };
  };

  inherit ingressSources;
}
