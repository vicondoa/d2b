{ lib, flakeRoot, ... }:

let
  identity = import (flakeRoot + "/nixos-modules/v2-identity.nix");
  stablePrincipalId =
    (import (flakeRoot + "/nixos-modules/lib.nix") { inherit lib; }).stablePrincipalId;

  realmId = path: identity.deriveRealmId path;
  homeId = realmId "home.local-root";
  devId = realmId "dev.local-root";
  workId = realmId "work.local-root";
  remoteId = realmId "remote.local-root";

  optionFixture =
    { lib, ... }:
    {
      options = {
        assertions = lib.mkOption {
          type = lib.types.listOf lib.types.attrs;
          default = [ ];
        };
        d2b.site = {
          launcherUsers = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
          };
          adminUsers = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
          };
        };
        users.groups = lib.mkOption {
          type = lib.types.attrsOf (
            lib.types.submodule {
              options.gid = lib.mkOption {
                type = lib.types.nullOr lib.types.int;
                default = null;
              };
            }
          );
          default = { };
        };
        users.users = lib.mkOption {
          type = lib.types.attrsOf (
            lib.types.submodule {
              options = {
                uid = lib.mkOption {
                  type = lib.types.nullOr lib.types.int;
                  default = null;
                };
                isSystemUser = lib.mkOption {
                  type = lib.types.bool;
                  default = false;
                };
                isNormalUser = lib.mkOption {
                  type = lib.types.bool;
                  default = false;
                };
                group = lib.mkOption {
                  type = lib.types.nullOr lib.types.str;
                  default = null;
                };
                extraGroups = lib.mkOption {
                  type = lib.types.listOf lib.types.str;
                  default = [ ];
                };
                description = lib.mkOption {
                  type = lib.types.str;
                  default = "";
                };
              };
            }
          );
          default = { };
        };
      };

      config = {
        d2b.site = {
          launcherUsers = [ "alice" ];
          adminUsers = [ "admin" ];
        };
        d2b.realms = {
          local-root = {
            path = "local-root";
            placement = "host-local";
            allowedUsers = [ ];
            allowedGroups = [ ];
          };
          home = {
            parent = "local-root";
            path = "home.local-root";
            placement = "host-local";
            allowedUsers = [ "alice" ];
            allowedGroups = [ "home-readers" ];
          };
          dev = {
            parent = "local-root";
            path = "dev.local-root";
            placement = "host-local";
            allowedUsers = [ "bob" ];
          };
          work = {
            parent = "local-root";
            path = "work.local-root";
            placement = "host-local";
            allowedUsers = [ "bob" ];
            allowedGroups = [ "work-readers" ];
          };
          remote = {
            parent = "local-root";
            path = "remote.local-root";
            placement = "gateway-vm";
            allowedUsers = [ "eve" ];
          };
        };
        users.users = lib.genAttrs [ "alice" "admin" "bob" "eve" ] (_: {
          isNormalUser = true;
        });
      };
    };

  evaluated = lib.evalModules {
    modules = [
      optionFixture
      (flakeRoot + "/nixos-modules/options-realms.nix")
      (flakeRoot + "/nixos-modules/index.nix")
      (flakeRoot + "/nixos-modules/host-users.nix")
    ];
  };
  cfg = evaluated.config;
  principalRows = cfg.d2b._realmPrincipals.children;
  accessRows = cfg.d2b._realmAccess.children;
  principalNames = lib.concatMap (row: [
    row.controller
    row.broker
    row.internalGroup
    row.publicGroup
  ]) principalRows;
  principalIds = map stablePrincipalId principalNames;
  serviceUsers = lib.filterAttrs (
    name: _: lib.hasPrefix "d2bd-r-" name || lib.hasPrefix "d2bbr-r-" name
  ) cfg.users.users;
  serviceUids = map (user: user.uid) (lib.attrValues serviceUsers);
  failedAssertions = lib.filter (entry: !entry.assertion) cfg.assertions;
  expectedNames = id: {
    controller = "d2bd-r-${id}";
    broker = "d2bbr-r-${id}";
    internalGroup = "d2bcg-r-${id}";
    publicGroup = "d2b-r-${id}";
  };
  rowById = cfg.d2b._realmPrincipals.byRealmId;
  accessById = cfg.d2b._realmAccess.byRealmId;
in
{
  "principal-uid-collision/no-uid-collision" = {
    expr = {
      moduleAssertions = builtins.length failedAssertions;
      distinctNames = builtins.length (lib.unique principalNames);
      distinctIds = builtins.length (lib.unique principalIds);
    };
    expected = {
      moduleAssertions = 0;
      distinctNames = 12;
      distinctIds = 12;
    };
  };

  "principal-uid-collision/all-uids-in-range" = {
    expr = lib.all (id: id >= 50000 && id <= 16827215) principalIds;
    expected = true;
  };

  "principal-uid-collision/no-out-of-range" = {
    expr = {
      home = lib.intersectAttrs (expectedNames homeId) rowById.${homeId};
      dev = lib.intersectAttrs (expectedNames devId) rowById.${devId};
      work = lib.intersectAttrs (expectedNames workId) rowById.${workId};
      remoteMaterialized = builtins.hasAttr remoteId rowById;
    };
    expected = {
      home = expectedNames homeId;
      dev = expectedNames devId;
      work = expectedNames workId;
      remoteMaterialized = false;
    };
  };

  "principal-uid-collision/no-user-uid-collision" = {
    expr = {
      serviceUserCount = builtins.length serviceUids;
      distinctServiceUids = builtins.length (lib.unique serviceUids);
      controllerGroups = map (row: cfg.users.users.${row.controller}.extraGroups) principalRows;
      brokerGroups = map (row: cfg.users.users.${row.broker}.extraGroups) principalRows;
      serviceInPublicGroup = lib.any (
        row:
        builtins.elem row.publicGroup cfg.users.users.${row.controller}.extraGroups
        || builtins.elem row.publicGroup cfg.users.users.${row.broker}.extraGroups
      ) principalRows;
    };
    expected = {
      serviceUserCount = 6;
      distinctServiceUids = 6;
      controllerGroups = map (row: [ row.internalGroup ]) principalRows;
      brokerGroups = map (row: [ row.internalGroup ]) principalRows;
      serviceInPublicGroup = false;
    };
  };

  "principal-uid-collision/principal-count" = {
    expr = {
      childCount = builtins.length principalRows;
      localRoot = cfg.d2b._realmPrincipals.localRoot;
      aliceGroups = lib.sort lib.lessThan cfg.users.users.alice.extraGroups;
      adminGroups = lib.sort lib.lessThan cfg.users.users.admin.extraGroups;
      bobGroups = lib.sort lib.lessThan cfg.users.users.bob.extraGroups;
      eveGroups = cfg.users.users.eve.extraGroups;
      legacyGroupPresent = builtins.hasAttr "d2b-launcher" cfg.users.groups;
    };
    expected = {
      childCount = 3;
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
      aliceGroups = lib.sort lib.lessThan [
        "d2b"
        "d2b-r-${homeId}"
      ];
      adminGroups = lib.sort lib.lessThan [
        "d2b-r-${devId}"
        "d2b-r-${homeId}"
        "d2b-r-${workId}"
      ];
      bobGroups = lib.sort lib.lessThan [
        "d2b-r-${devId}"
        "d2b-r-${workId}"
      ];
      eveGroups = [ ];
      legacyGroupPresent = false;
    };
  };

  "principal-uid-collision/distinct-uid-count" = {
    expr = {
      state = accessById.${homeId}.resources.state;
      audit = accessById.${homeId}.resources.audit;
      runtime = accessById.${homeId}.resources.runtime;
      publicSocket = accessById.${homeId}.resources.publicSocket;
      brokerSocket = accessById.${homeId}.resources.brokerSocket;
    };
    expected = {
      state = {
        path = "/var/lib/d2b/r/${homeId}";
        owner = "d2bd-r-${homeId}";
        group = "d2bd-r-${homeId}";
        mode = "0750";
        repairOwner = "d2bbr-r-${homeId}";
      };
      audit = {
        path = "/var/lib/d2b/r/${homeId}/audit";
        owner = "d2bbr-r-${homeId}";
        group = "d2bbr-r-${homeId}";
        mode = "0750";
        repairOwner = "d2bbr-r-${homeId}";
      };
      runtime = {
        path = "/run/d2b/r/${homeId}";
        owner = "root";
        group = "d2bcg-r-${homeId}";
        mode = "0750";
        acl = [
          "g:d2b-r-${homeId}:--x"
          "g:home-readers:--x"
        ];
        repairOwner = "root";
      };
      publicSocket = {
        acceptor = "d2bd-r-${homeId}";
        owner = "d2bd-r-${homeId}";
        group = "d2b-r-${homeId}";
        mode = "0660";
        path = "/run/d2b/r/${homeId}/public.sock";
        acl = [ "g:home-readers:rw-" ];
        repairOwner = "root";
      };
      brokerSocket = {
        acceptor = "d2bbr-r-${homeId}";
        client = "d2bd-r-${homeId}";
        owner = "d2bbr-r-${homeId}";
        group = "d2bd-r-${homeId}";
        mode = "0660";
        path = "/run/d2b/r/${homeId}/broker.sock";
        acl = [ ];
        repairOwner = "root";
      };
    };
  };
}
