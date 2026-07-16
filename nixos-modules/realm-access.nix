{ config, lib, ... }:

let
  cfg = config.d2b;
  childRows = cfg._realmPrincipals.children or [ ];
  adminUsers = cfg.site.adminUsers or [ ];

  accessUsers = row: lib.sort lib.lessThan (lib.unique (row.allowedUsers ++ adminUsers));

  mkAccessRow =
    row:
    let
      stateRoot = "/var/lib/d2b/r/${row.realmId}";
      runtimeRoot = "/run/d2b/r/${row.realmId}";
      cacheRoot = "/var/cache/d2b/r/${row.realmId}";
      publicGroupAcls = map (group: "g:${group}:rw-") row.allowedGroups;
      traversalAcls = [ "g:${row.publicGroup}:--x" ] ++ map (group: "g:${group}:--x") row.allowedGroups;
    in
    row
    // {
      authorizedUsers = accessUsers row;
      resources = {
        state = {
          path = stateRoot;
          owner = row.controller;
          group = row.controller;
          mode = "0750";
          repairOwner = row.broker;
        };
        controller = {
          path = "${stateRoot}/controller";
          owner = row.controller;
          group = row.controller;
          mode = "0700";
          repairOwner = row.broker;
        };
        broker = {
          path = "${stateRoot}/broker";
          owner = row.broker;
          group = row.broker;
          mode = "0700";
          repairOwner = row.broker;
        };
        audit = {
          path = "${stateRoot}/audit";
          owner = row.broker;
          group = row.broker;
          mode = "0750";
          repairOwner = row.broker;
        };
        cache = {
          path = cacheRoot;
          owner = row.controller;
          group = row.controller;
          mode = "0750";
          repairOwner = row.broker;
        };
        runtime = {
          path = runtimeRoot;
          owner = "root";
          group = row.internalGroup;
          mode = "0750";
          acl = traversalAcls;
          repairOwner = cfg._realmPrincipals.localRoot.broker;
        };
        publicSocket = row.socketPrincipals.public // {
          path = "${runtimeRoot}/public.sock";
          acl = publicGroupAcls;
          repairOwner = cfg._realmPrincipals.localRoot.broker;
        };
        brokerSocket = row.socketPrincipals.broker // {
          path = "${runtimeRoot}/broker.sock";
          acl = [ ];
          repairOwner = cfg._realmPrincipals.localRoot.broker;
        };
      };
    };

  rows = map mkAccessRow childRows;
in
{
  options.d2b._realmAccess = lib.mkOption {
    type = lib.types.attrs;
    internal = true;
    visible = false;
    readOnly = true;
  };

  config = {
    d2b._realmAccess = {
      localRoot = {
        inherit (cfg._realmPrincipals.localRoot)
          controller
          broker
          internalGroup
          publicGroup
          socketPrincipals
          ;
        resources = {
          state = "/var/lib/d2b/host";
          cache = "/var/cache/d2b/host";
          runtime = "/run/d2b";
          audit = "/var/lib/d2b/host/audit";
        };
      };
      children = rows;
      byRealmId = lib.listToAttrs (map (row: lib.nameValuePair row.realmId row) rows);
    };

    users.users = lib.mkMerge (
      map (
        row:
        lib.genAttrs (accessUsers row) (_: {
          extraGroups = [ row.publicGroup ];
        })
      ) childRows
    );
  };
}
