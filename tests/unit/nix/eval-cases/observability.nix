{ flakeRoot }:

let
  flake = builtins.getFlake "git+file://${toString flakeRoot}";
  lib = flake.inputs.nixpkgs.lib;
  identity = import (flakeRoot + "/nixos-modules/v2-identity.nix");
  realmId = identity.deriveRealmId "local-root";
  workloadId = identity.deriveWorkloadId realmId "sys-obs";
  config.d2b = {
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
        configuredName = "sys-obs";
        canonicalTarget = "sys-obs.local-root.d2b";
        realmPath = "local-root";
        inherit realmId workloadId;
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
  rows = import (flakeRoot + "/nixos-modules/realm-observability-rows.nix") {
    inherit config lib;
  };
in
{
  enabled = rows.enabled;
  inherit (rows) schemaVersion;
  inherit (rows.workload) canonicalTarget realmId workloadId;
  provider = rows.projections.provider;
  projection = {
    inherit (rows.projections.policy)
      bounded rawAuditAccess rawRepairStateAccess redaction;
  };
  canonicalPaths = lib.all
    (row:
      lib.hasInfix "/r/${realmId}/" row.path
      && !(lib.hasInfix "/vms/" row.path))
    rows.paths;
  secretCount = builtins.length rows.secrets;
}
