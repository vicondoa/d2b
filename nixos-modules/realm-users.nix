{ config, lib, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  identity = import ./v2-identity.nix;
  inherit (d2bLib) stablePrincipalId;

  isLocalRoot = realm: realm.realmPath == "local-root";

  childRealms = lib.filter (
    realm: realm.placement == "host-local" && !(isLocalRoot realm)
  ) cfg._index.realms.enabledList;

  mkPrincipalRow =
    realm:
    let
      realmId = identity.validateShortId realm.realmId;
      declaredRealm = cfg.realms.${realm.realmName};
      controller = "d2bd-r-${realmId}";
      broker = "d2bbr-r-${realmId}";
      internalGroup = "d2bcg-r-${realmId}";
      publicGroup = "d2b-r-${realmId}";
    in
    {
      inherit
        realmId
        controller
        broker
        internalGroup
        publicGroup
        ;
      realmPath = realm.realmPath;
      allowedUsers = lib.sort lib.lessThan (lib.unique declaredRealm.allowedUsers);
      allowedGroups = lib.sort lib.lessThan (lib.unique declaredRealm.allowedGroups);
      socketPrincipals = {
        public = {
          acceptor = controller;
          owner = controller;
          group = publicGroup;
          mode = "0660";
        };
        broker = {
          acceptor = broker;
          client = controller;
          owner = broker;
          group = controller;
          mode = "0660";
        };
      };
    };

  childRows = map mkPrincipalRow childRealms;
  childIds = map (row: row.realmId) childRows;
  byRealmId = lib.listToAttrs (map (row: lib.nameValuePair row.realmId row) childRows);

  groupRows = lib.concatMap (
    row:
    map
      (name: {
        inherit name;
        id = stablePrincipalId name;
      })
      [
        row.controller
        row.broker
        row.internalGroup
        row.publicGroup
      ]
  ) childRows;
  userRows = lib.concatMap (
    row:
    map
      (name: {
        inherit name;
        id = stablePrincipalId name;
      })
      [
        row.controller
        row.broker
      ]
  ) childRows;
  numericRows = map (
    name: builtins.head (lib.filter (row: row.name == name) (groupRows ++ userRows))
  ) (lib.unique (map (row: row.name) (groupRows ++ userRows)));
  numericIds = lib.unique (map (row: row.id) numericRows);
in
{
  options.d2b._realmPrincipals = lib.mkOption {
    type = lib.types.attrs;
    internal = true;
    visible = false;
    readOnly = true;
  };

  config = {
    d2b._realmPrincipals = {
      localRoot = {
        realmId = identity.deriveRealmId "local-root";
        controller = "d2bd";
        broker = "root";
        internalGroup = null;
        publicGroup = "d2b";
        socketPrincipals = {
          public = {
            acceptor = "d2bd";
            owner = "d2bd";
            group = "d2b";
            mode = "0660";
          };
          broker = {
            acceptor = "root";
            client = "d2bd";
            owner = "root";
            group = "d2bd";
            mode = "0660";
          };
        };
      };
      children = childRows;
      inherit byRealmId;
    };

    users.groups = lib.listToAttrs (
      lib.concatMap (
        row:
        map
          (
            name:
            lib.nameValuePair name {
              gid = stablePrincipalId name;
            }
          )
          [
            row.controller
            row.broker
            row.internalGroup
            row.publicGroup
          ]
      ) childRows
    );

    users.users = lib.listToAttrs (
      lib.concatMap (row: [
        (lib.nameValuePair row.controller {
          isSystemUser = true;
          uid = stablePrincipalId row.controller;
          group = row.controller;
          extraGroups = [ row.internalGroup ];
          description = "d2b child realm controller ${row.realmId}";
        })
        (lib.nameValuePair row.broker {
          isSystemUser = true;
          uid = stablePrincipalId row.broker;
          group = row.broker;
          extraGroups = [ row.internalGroup ];
          description = "d2b child realm broker ${row.realmId}";
        })
      ]) childRows
    );

    assertions = [
      {
        assertion = builtins.length childIds == builtins.length (lib.unique childIds);
        message = "d2b realm principal collision: canonical realm IDs must be unique";
      }
      {
        assertion = builtins.length numericRows == builtins.length numericIds;
        message = "d2b realm principal collision: stable UID/GID allocation must be unique";
      }
    ];
  };
}
