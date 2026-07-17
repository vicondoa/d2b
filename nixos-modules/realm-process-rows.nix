{ config, lib, ... }:

let
  cfg = config.d2b;
  childRealms = lib.sortOn (row: row.realmPath) (cfg._realmAccess.children or [ ]);
  namespaceKinds = [
    "user"
    "mount"
    "network"
    "ipc"
    "pid"
    "cgroup"
  ];

  endpointFor =
    realmId: endpointKind:
    lib.findFirst (
      endpoint: endpoint.realmId == realmId && endpoint.endpointKind == endpointKind
    ) (throw "missing ${endpointKind} endpoint row for realm ${realmId}") cfg._realmEndpointRows;

  mkProcess =
    row: processRole:
    let
      isController = processRole == "controller";
      endpointKind = if isController then "public" else "broker";
      endpoint = endpointFor row.realmId endpointKind;
      principal = if isController then row.controller else row.broker;
      cgroupLeaf = "/sys/fs/cgroup/d2b.slice/r-${row.realmId}/${processRole}";
      namespaceRefs = map (
        namespaceKind: "realm-${row.realmId}-namespace-${processRole}-${namespaceKind}"
      ) namespaceKinds;
      baseResourceRefs = [
        "realm-${row.realmId}-cgroup-${processRole}"
        "realm-${row.realmId}-state"
        "realm-${row.realmId}-runtime"
        "realm-${row.realmId}-listener-${endpointKind}"
      ];
      roleResourceRefs =
        if isController
        then [ "realm-${row.realmId}-cache" ]
        else [ "realm-${row.realmId}-audit" ];
    in
    {
      launchId = "realm-${row.realmId}-${processRole}-launch";
      realmId = row.realmId;
      realmPath = row.realmPath;
      inherit processRole principal cgroupLeaf namespaceRefs;
      executable = if isController then "d2bd" else "d2b-priv-broker";
      group = principal;
      supplementaryGroups = [ row.internalGroup ];
      listenerRef = endpoint.endpointId;
      listenerFdName = endpoint.fdName;
      configRef =
        if isController
        then "/etc/d2b/r/${row.realmId}/controller.json"
        else "/etc/d2b/r/${row.realmId}/broker.json";
      identityConfigRef = "/etc/d2b/realm-identity.json";
      resourceRefs = baseResourceRefs ++ namespaceRefs ++ roleResourceRefs;
      spawnAuthority = "local-root-broker";
      supervisionOwner = "local-root-controller";
      parentSpawnRequired = true;
      initialCgroupPlacement = "direct";
      receivesSystemdListenFds = false;
      selfBindsListener = false;
      declarativeOnly = true;
    };

  rows = lib.concatMap (
    row:
    map (processRole: mkProcess row processRole) [
      "controller"
      "broker"
    ]
  ) childRealms;
in
{
  options.d2b._realmProcessRows = lib.mkOption {
    type = lib.types.listOf lib.types.attrs;
    internal = true;
    visible = false;
    readOnly = true;
  };

  config.d2b._realmProcessRows = rows;
}
