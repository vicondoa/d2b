{ config, lib, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  lifecycleUsers = lib.unique (cfg.site.adminUsers ++ cfg.site.launcherUsers);
in
{
  users.groups = {
    # Membership grants admission to the root daemon public socket. Object
    # authorization remains the daemon's SO_PEERCRED and Zone policy check.
    d2b = { };

    # The broker-resolved Zone store owner is a real host principal, not a
    # product hierarchy. Keep its stable numeric identity for restart repair.
    d2b-zonert = {
      gid = d2bLib.stablePrincipalId "d2b-zonert";
    };
  };

  users.users = lib.mkMerge [
    (lib.genAttrs lifecycleUsers (_: {
      extraGroups = [ "d2b" ];
    }))
    {
      d2b-zonert = {
        isSystemUser = true;
        uid = d2bLib.stablePrincipalId "d2b-zonert";
        group = "d2b-zonert";
        description = "d2b Zone resource-store owner";
      };
    }
  ];
}
