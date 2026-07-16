{ config, lib, ... }:

let
  identity = import ./v2-identity.nix;
  normalizeRealms = import ./index-realms.nix { inherit identity lib; };
  normalizeWorkloads = import ./index-workloads.nix { inherit identity lib; };
  normalizeResources = import ./index-resources.nix { inherit identity lib; };

  realmIndex = normalizeRealms config.d2b.realms;
  workloadIndex = normalizeWorkloads {
    realms = config.d2b.realms;
    inherit realmIndex;
  };
  resourceIndex = normalizeResources {
    realms = config.d2b.realms;
    inherit realmIndex workloadIndex;
  };

  enrichWorkload = row:
    let
      resources = resourceIndex.resources.byWorkloadId.${row.workloadId} or [ ];
    in
    row // {
      providerBindings =
        resourceIndex.providers.bindingsByWorkloadId.${row.workloadId} or { };
      roles = resourceIndex.roles.byWorkloadId.${row.workloadId} or [ ];
      inherit resources;
      storageIds = map (resource: resource.resourceId) resources;
    };
  workloadRows = map enrichWorkload workloadIndex.list;
  enabledWorkloadRows = lib.filter (row: row.enabled) workloadRows;
  workloadBy = field: rows:
    lib.listToAttrs (map (row: {
      name = row.${field};
      value = row;
    }) rows);
  workloadsByRealm = rows:
    lib.groupBy (row: row.realmId) rows;
  workloads = workloadIndex // {
    list = workloadRows;
    enabledList = enabledWorkloadRows;
    byId = workloadBy "workloadId" workloadRows;
    byCanonicalTarget = workloadBy "canonicalTarget" workloadRows;
    byRealmId = workloadsByRealm workloadRows;
    enabledByRealmId = workloadsByRealm enabledWorkloadRows;
  };

  index = {
    schemaVersion = 2;
    realms = realmIndex;
    inherit workloads;
    inherit (resourceIndex) providers roles resources storage;
    identities = {
      realmIds = map (row: row.realmId) realmIndex.list;
      workloadIds = map (row: row.workloadId) workloadIndex.list;
      providerIds = map (row: row.providerId) resourceIndex.providers.list;
      roleIds = map (row: row.roleId) resourceIndex.roles.list;
    };
  };
in
{
  options.d2b._index = lib.mkOption {
    type = lib.types.attrs;
    default = { };
    internal = true;
    visible = false;
    description = "Internal recursion-safe normalized realm, workload, provider, role, storage, and resource index.";
  };

  config.d2b._index = index;
}
