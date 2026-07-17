{ config, lib, ... }:

let
  cfg = config.d2b;
  childRealms = lib.sortOn (row: row.realmPath) (cfg._realmAccess.children or [ ]);
  processRoles = [
    "controller"
    "broker"
  ];
  namespaceKinds = [
    "user"
    "mount"
    "network"
    "ipc"
    "pid"
    "cgroup"
  ];

  acquisitionOrder = phase: ordinal: {
    inherit phase ordinal;
  };

  mkResource =
    {
      row,
      suffix,
      kind,
      share ? "exclusive",
      phase,
      ordinal,
      sourceKind,
      refName,
      delegation,
    }:
    {
      resourceId = "realm-${row.realmId}-${suffix}";
      realmId = row.realmId;
      realmPath = row.realmPath;
      inherit
        kind
        share
        delegation
        ;
      acquisitionOrder = acquisitionOrder phase ordinal;
      source = {
        kind = sourceKind;
        inherit refName;
      };
    };

  cgroupResources = row: [
    (mkResource {
      inherit row;
      suffix = "cgroup-root";
      kind = "cgroup-subtree";
      phase = 10;
      ordinal = 0;
      sourceKind = "realm-broker";
      refName = row.realmId;
      delegation = "directory-fd";
    })
    (mkResource {
      inherit row;
      suffix = "cgroup-controller";
      kind = "cgroup-subtree";
      phase = 10;
      ordinal = 1;
      sourceKind = "realm-broker";
      refName = row.realmId;
      delegation = "directory-fd";
    })
    (mkResource {
      inherit row;
      suffix = "cgroup-broker";
      kind = "cgroup-subtree";
      phase = 10;
      ordinal = 2;
      sourceKind = "realm-broker";
      refName = row.realmId;
      delegation = "directory-fd";
    })
    (mkResource {
      inherit row;
      suffix = "cgroup-workloads";
      kind = "cgroup-subtree";
      phase = 10;
      ordinal = 3;
      sourceKind = "realm-broker";
      refName = row.realmId;
      delegation = "directory-fd";
    })
  ];

  namespaceResources =
    row:
    lib.concatMap (
      processRole:
      lib.imap0 (
        namespaceOrdinal: namespaceKind:
        mkResource {
          inherit row;
          suffix = "namespace-${processRole}-${namespaceKind}";
          kind = "namespace-boundary";
          phase = if processRole == "controller" then 20 else 21;
          ordinal = namespaceOrdinal;
          sourceKind =
            if namespaceKind == "network"
            then "realm-network"
            else "realm-broker";
          refName = row.realmId;
          delegation = "namespace-fd";
        }
      ) namespaceKinds
    ) processRoles;

  pathResources = row: [
    (mkResource {
      inherit row;
      suffix = "state";
      kind = "host-file-partition";
      phase = 30;
      ordinal = 0;
      sourceKind = "realm-state-dir";
      refName = row.resources.state.path;
      delegation = "directory-fd";
    })
    (mkResource {
      inherit row;
      suffix = "cache";
      kind = "host-file-partition";
      phase = 30;
      ordinal = 1;
      sourceKind = "realm-state-dir";
      refName = row.resources.cache.path;
      delegation = "directory-fd";
    })
    (mkResource {
      inherit row;
      suffix = "runtime";
      kind = "host-file-partition";
      phase = 30;
      ordinal = 2;
      sourceKind = "realm-run-dir";
      refName = row.resources.runtime.path;
      delegation = "directory-fd";
    })
    (mkResource {
      inherit row;
      suffix = "audit";
      kind = "host-file-partition";
      phase = 30;
      ordinal = 3;
      sourceKind = "realm-audit-dir";
      refName = row.resources.audit.path;
      delegation = "directory-fd";
    })
  ];

  listenerResources =
    row:
    lib.imap0 (
      ordinal: endpoint:
      mkResource {
        inherit row;
        suffix = "listener-${endpoint.endpointKind}";
        kind = "host-file-partition";
        phase = 40;
        inherit ordinal;
        sourceKind = "realm-socket";
        refName = endpoint.path;
        delegation = "listener-fd";
      }
    ) (lib.filter (endpoint: endpoint.realmId == row.realmId) cfg._realmEndpointRows);

  resourcesFor =
    row:
    cgroupResources row
    ++ namespaceResources row
    ++ pathResources row
    ++ listenerResources row;

  resourceRows = lib.concatMap resourcesFor childRealms;
  leaseRequests = map (
    row:
    let
      resources = map (
        resource: {
          inherit (resource)
            resourceId
            kind
            share
            acquisitionOrder
            ;
        }
      ) (resourcesFor row);
    in
    {
      requestId = "realm-${row.realmId}-bootstrap-lease";
      realmId = row.realmId;
      realmPath = row.realmPath;
      controllerGenerationRef = "realm-${row.realmId}-controller-generation";
      resourceIds = map (resource: resource.resourceId) resources;
      inherit resources;
      typed = true;
      declarativeOnly = true;
    }
  ) childRealms;
in
{
  options.d2b._realmResourceRows = lib.mkOption {
    type = lib.types.attrs;
    internal = true;
    visible = false;
    readOnly = true;
  };

  config.d2b._realmResourceRows = {
    resources = resourceRows;
    inherit leaseRequests;
  };
}
