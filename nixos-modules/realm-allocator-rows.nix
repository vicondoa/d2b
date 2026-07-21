{ config, lib, ... }:

let
  cfg = config.d2b;
  childRealms = lib.sortOn (row: row.realmPath) (cfg._realmAccess.children or [ ]);
  childRealmIds = map (row: row.realmId) childRealms;
  namespaceKinds = [
    "user"
    "mount"
    "network"
    "ipc"
    "pid"
    "cgroup"
  ];
  processRoles = [
    "controller"
    "broker"
  ];

  workloadsFor = realmId: cfg._index.workloads.enabledByRealmId.${realmId} or [ ];
  rolesFor = workloadId: cfg._index.roles.byWorkloadId.${workloadId} or [ ];

  cgroupRowsFor =
    row:
    let
      root = "/sys/fs/cgroup/d2b.slice/r-${row.realmId}";
      workloadRows = workloadsFor row.realmId;
    in
    [
      {
        cgroupId = "realm-${row.realmId}-root";
        path = root;
        kind = "realm-root";
        realmId = row.realmId;
        realmPath = row.realmPath;
        owner = "root";
        group = row.internalGroup;
        processFree = true;
        delegated = true;
      }
      {
        cgroupId = "realm-${row.realmId}-controller";
        path = "${root}/controller";
        kind = "controller-leaf";
        realmId = row.realmId;
        realmPath = row.realmPath;
        owner = row.controller;
        group = row.internalGroup;
        processFree = false;
        delegated = true;
      }
      {
        cgroupId = "realm-${row.realmId}-broker";
        path = "${root}/broker";
        kind = "broker-leaf";
        realmId = row.realmId;
        realmPath = row.realmPath;
        owner = row.broker;
        group = row.internalGroup;
        processFree = false;
        delegated = true;
      }
      {
        cgroupId = "realm-${row.realmId}-workloads";
        path = "${root}/workloads";
        kind = "workloads-root";
        realmId = row.realmId;
        realmPath = row.realmPath;
        owner = row.controller;
        group = row.internalGroup;
        processFree = true;
        delegated = true;
      }
    ]
    ++ lib.concatMap (
      workload:
      let
        workloadRoot = "${root}/workloads/w-${workload.workloadId}";
      in
      [
        {
          cgroupId = "workload-${workload.workloadId}-root";
          path = workloadRoot;
          kind = "workload-root";
          realmId = row.realmId;
          realmPath = row.realmPath;
          workloadId = workload.workloadId;
          owner = row.controller;
          group = row.internalGroup;
          processFree = true;
          delegated = true;
        }
      ]
      ++ map (role: {
        cgroupId = "role-${role.roleId}";
        path = "${workloadRoot}/${role.roleId}";
        kind = "role-leaf";
        realmId = row.realmId;
        realmPath = row.realmPath;
        workloadId = workload.workloadId;
        roleId = role.roleId;
        owner = row.controller;
        group = row.internalGroup;
        processFree = false;
        delegated = true;
      }) (rolesFor workload.workloadId)
    ) workloadRows;

  cgroupRows = lib.concatMap cgroupRowsFor childRealms;

  namespaceRows = lib.concatMap (
    row:
    lib.concatMap (
      processRole:
      map (namespaceKind: {
        namespaceId = "realm-${row.realmId}-namespace-${processRole}-${namespaceKind}";
        realmId = row.realmId;
        realmPath = row.realmPath;
        inherit processRole namespaceKind;
        owner = if processRole == "controller" then row.controller else row.broker;
        userMapPrincipal =
          if processRole == "controller" then row.controller else row.broker;
        mappedInternalGroup = row.internalGroup;
        initialNamespaceAuthority = false;
        dedicated = true;
      }) namespaceKinds
    ) processRoles
  ) childRealms;

  identityConfigRows = lib.concatMap (
    row:
    map (
      processRole:
      let
        principal = if processRole == "controller" then row.controller else row.broker;
      in
      {
        identityConfigId = "realm-${row.realmId}-${processRole}-identity";
        realmId = row.realmId;
        realmPath = row.realmPath;
        inherit processRole principal;
        primaryGroup = principal;
        supplementaryGroups = [ row.internalGroup ];
        uidMap = [
          {
            insideId = 0;
            outsideId = config.users.users.${principal}.uid;
            length = 1;
          }
        ];
        gidMap = [
          {
            insideId = 0;
            outsideId = config.users.groups.${principal}.gid;
            length = 1;
          }
          {
            insideId = 1;
            outsideId = config.users.groups.${row.internalGroup}.gid;
            length = 1;
          }
        ];
        initialNamespaceCapabilitiesEmpty = true;
      }
    ) processRoles
  ) childRealms;

  ownershipRows = lib.concatMap (
    row:
    let
      accessResources = lib.mapAttrsToList (
        resourceKind: resource:
        {
          ownershipId = "realm-${row.realmId}-${resourceKind}";
          realmId = row.realmId;
          realmPath = row.realmPath;
          inherit resourceKind;
          inherit (resource)
            path
            owner
            group
            repairOwner
            ;
          mode = resource.mode or null;
          acl = resource.acl or [ ];
        }
      ) row.resources;
      cgroups = map (cgroup: {
        ownershipId = cgroup.cgroupId;
        realmId = row.realmId;
        realmPath = row.realmPath;
        resourceKind = "cgroup";
        inherit (cgroup) path owner group;
        repairOwner = "local-root-broker";
        mode = null;
        acl = [ ];
      }) (lib.filter (cgroup: cgroup.realmId == row.realmId) cgroupRows);
    in
    accessResources ++ cgroups
  ) childRealms;

  acquisitionRows = lib.concatMap (
    request:
    map (resource: {
      inherit (request) realmId realmPath requestId;
      inherit (resource)
        resourceId
        kind
        acquisitionOrder
        ;
    }) request.resources
  ) cfg._realmResourceRows.leaseRequests;

  processLaunchOrder = lib.imap0 (
    ordinal: process:
    {
      inherit ordinal;
      inherit (process)
        launchId
        realmId
        realmPath
        processRole
        ;
    }
  ) cfg._realmProcessRows;
in
{
  imports = [
    ./realm-endpoint-rows.nix
    ./realm-resource-rows.nix
    ./realm-process-rows.nix
  ];

  options.d2b._realmAllocatorRows = lib.mkOption {
    type = lib.types.attrs;
    internal = true;
    visible = false;
    readOnly = true;
  };

  config = {
    d2b._realmAllocatorRows = {
      schemaVersion = 1;
      endpoints = cfg._realmEndpointRows;
      resources = cfg._realmResourceRows.resources;
      leaseRequests = cfg._realmResourceRows.leaseRequests;
      processes = cfg._realmProcessRows;
      cgroups = cgroupRows;
      namespaces = namespaceRows;
      identityConfigs = identityConfigRows;
      ownership = ownershipRows;
      ordering = {
        resourceAcquisition = acquisitionRows;
        processLaunch = processLaunchOrder;
      };
      invariants = {
        declarativeOnly = true;
        childUnitsEmitted = false;
        listenerBindingPerformed = false;
        processSpawnPerformed = false;
        leaseExecutionPerformed = false;
        realmRootsProcessFree = true;
        workloadInteriorsProcessFree = true;
      };
    };

    assertions = [
      {
        assertion =
          builtins.length childRealmIds
          == builtins.length (lib.unique childRealmIds);
        message = "d2b allocator rows require unique child realm IDs";
      }
      {
        assertion = lib.all (
          request: builtins.length request.resources <= 32
        ) cfg._realmResourceRows.leaseRequests;
        message = "d2b allocator lease request exceeds the closed 32-resource bound";
      }
      {
        assertion = lib.all (
          row:
          row.kind != "realm-root"
          || row.processFree
        ) cgroupRows;
        message = "d2b allocator realm cgroup roots must remain process-free";
      }
    ];
  };
}
