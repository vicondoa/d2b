{ lib, flakeRoot, ... }:

let
  identity = import (flakeRoot + "/nixos-modules/v2-identity.nix");
  realmId = identity.deriveRealmId "local-root";
  stackWorkloadId = identity.deriveWorkloadId realmId "sys-obs";
  workRealmId = identity.deriveRealmId "work.local-root";
  workWorkloadId = identity.deriveWorkloadId workRealmId "work-app";
  personalRealmId = identity.deriveRealmId "personal.local-root";
  personalWorkloadId =
    identity.deriveWorkloadId personalRealmId "work-app";

  config = {
    d2b = {
      observability = {
        enable = true;
        vmName = "sys-obs";
        host = {
          identityName = "demo";
          otlpIngest.clientGroup = null;
        };
        signoz = {
          otlpGrpcPort = 4317;
          otlpHttpPort = 4318;
          jwtSecretFile = null;
          rootPasswordFile = null;
          clickhousePasswordFile = null;
        };
      };
      _index.workloads.enabledList = [
        {
          enabled = true;
          workloadId = stackWorkloadId;
          configuredName = "sys-obs";
          canonicalTarget = "sys-obs.local-root.d2b";
          inherit realmId;
          realmPath = "local-root";
        }
        {
          enabled = true;
          workloadId = workWorkloadId;
          configuredName = "work-app";
          canonicalTarget = "work-app.work.local-root.d2b";
          realmId = workRealmId;
          realmPath = "work.local-root";
        }
        {
          enabled = true;
          workloadId = personalWorkloadId;
          configuredName = "work-app";
          canonicalTarget = "work-app.personal.local-root.d2b";
          realmId = personalRealmId;
          realmPath = "personal.local-root";
        }
      ];
      _bundle.providerRegistryV2Json.data.providers = [
        {
          descriptor.providerId =
            identity.deriveProviderId
              realmId "observability" "observability-local";
          binding = {
            axis = "local-observability";
            maxRecords = 64;
            maxBytes = 32768;
            maxTimeWindowMs = 86400000;
          };
        }
      ];
    };
  };
  rows = import (flakeRoot + "/nixos-modules/realm-observability-rows.nix") {
    inherit config lib;
  };
  source = lib.findFirst
    (row: row.canonicalTarget == "work-app.work.local-root.d2b")
    (throw "missing work observability source")
    rows.projections.policy.sourceRows;
  secretRoot =
    "/var/lib/d2b/r/${realmId}/w/${stackWorkloadId}/observability/secrets/";
in
{
  "observability/realm-workload-identity" = {
    expr = rows.workload;
    expected = {
      inherit realmId;
      workloadId = stackWorkloadId;
      runtimeProviderId =
        identity.deriveProviderId realmId "runtime" "runtime-local";
      configuredName = "sys-obs";
      canonicalTarget = "sys-obs.local-root.d2b";
      autostart = true;
    };
  };

  "observability/canonical-resources" = {
    expr = {
      hostEgress = rows.endpoints.hostEgress.path;
      hostEgressRole = rows.endpoints.hostEgress.roleId;
      hostEgressOwner = rows.endpoints.hostEgress.owner;
      hostEgressClients = rows.endpoints.hostEgress.clients;
      hostIngest = rows.endpoints.hostIngest.path;
      stackVsock = rows.endpoints.stackVsock.path;
      pathsAreCanonical = lib.all
        (row:
          lib.hasInfix "/r/${realmId}/" row.path
          && !(lib.hasInfix "/vms/" row.path))
        rows.paths;
      brokerOwnsPaths =
        lib.all (row: row.creator == "realm-broker") rows.paths;
    };
    expected = {
      hostEgress =
        "/run/d2b/r/${realmId}/w/${stackWorkloadId}/roles/${
          identity.deriveRoleId realmId stackWorkloadId "vsock-relay"
        }/host-egress.sock";
      hostEgressRole =
        identity.deriveRoleId realmId stackWorkloadId "vsock-relay";
      hostEgressOwner = "realm-broker";
      hostEgressClients = [ "d2b-host-otel-collector" ];
      hostIngest =
        "/run/d2b/r/${realmId}/w/${stackWorkloadId}/sockets/ingest/host-otlp.sock";
      stackVsock =
        "/var/lib/d2b/r/${realmId}/w/${stackWorkloadId}/vsock.sock";
      pathsAreCanonical = true;
      brokerOwnsPaths = true;
    };
  };

  "observability/same-name-sources-use-canonical-ids" = {
    expr = {
      sourceKeys = lib.sort lib.lessThan
        (lib.remove "host" (lib.attrNames rows.ingressSources));
      sourceTargets = map (row: row.canonicalTarget)
        rows.projections.policy.sourceRows;
    };
    expected = {
      sourceKeys = lib.sort lib.lessThan [
        personalWorkloadId
        workWorkloadId
      ];
      sourceTargets = [
        "work-app.personal.local-root.d2b"
        "work-app.work.local-root.d2b"
      ];
    };
  };

  "observability/frozen-provider-mapping-is-consumed" = {
    expr = rows.projections.provider;
    expected = {
      providerId =
        identity.deriveProviderId
          realmId "observability" "observability-local";
      axis = "local-observability";
      registration = "frozen-provider-registry-v2";
      limits = {
        maxRecords = 64;
        maxBytes = 32768;
        maxTimeWindowMs = 86400000;
      };
    };
  };

  "observability/projections-stay-bounded-and-redacted" = {
    expr = {
      inherit (rows.projections.policy)
        bounded rawAuditAccess rawRepairStateAccess redaction;
      sourceTarget = source.canonicalTarget;
      allowed = source.projection.fields;
      excluded = source.projection.excluded;
    };
    expected = {
      bounded = true;
      rawAuditAccess = false;
      rawRepairStateAccess = false;
      redaction = "positive-allowlist";
      sourceTarget = "work-app.work.local-root.d2b";
      allowed = [
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
  };

  "observability/secrets-are-declarative-workload-resources" = {
    expr = {
      count = builtins.length rows.secrets;
      brokerOwned = lib.all
        (secret:
          secret.owner == "realm-broker"
          && lib.hasPrefix secretRoot secret.path)
        rows.secrets;
      generatedSizes = map (secret: secret.generatedBytes) rows.secrets;
      modes = lib.unique (map (secret: secret.mode) rows.secrets);
    };
    expected = {
      count = 3;
      brokerOwned = true;
      generatedSizes = [ 64 48 48 ];
      modes = [ "0400" ];
    };
  };

  "observability/no-second-registration-path" = {
    expr =
      let
        moduleText = builtins.readFile
          (flakeRoot + "/nixos-modules/realm-observability-rows.nix");
      in {
        fragmentAbsent = !(builtins.pathExists
          (flakeRoot
            + "/nixos-modules/provider-registry-v2-extensions/observability.nix"));
        emitsProviders = lib.hasInfix "providers =" moduleText;
        referencesFrozenRegistration =
          lib.hasInfix "frozen-provider-registry-v2" moduleText;
      };
    expected = {
      fragmentAbsent = true;
      emitsProviders = false;
      referencesFrozenRegistration = true;
    };
  };
}
