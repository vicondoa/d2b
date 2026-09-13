{ config, lib, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  lifecycleUsers = lib.unique (cfg.site.adminUsers ++ cfg.site.launcherUsers);

  # The per-Device TPM principals a zone-native Guest's Device needs: the
  # state Volume grants both through ACL entries
  # (`User/d2b-<zone>-<device>-swtpm` and its `-flush` sibling), and the
  # worker rows run as those principals' uids. The directory's layout owner is
  # the daemon itself (`User/d2bd`), which creates it and cannot chown it to
  # another uid. Both accounts are derived from the trusted bundle rows
  # (`d2bLib.deviceTpmPrincipals`), so the accounts the Volume grants access to
  # are the accounts its workers actually run as.
  tpmPrincipals = d2bLib.deviceTpmPrincipals cfg;
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
  } // (lib.genAttrs (map (row: row.account) tpmPrincipals) (account: {
    gid = (lib.findFirst (row: row.account == account) null tpmPrincipals).ownerUid;
  })) // (lib.genAttrs (map (row: row.flushAccount) tpmPrincipals) (account: {
    gid = (lib.findFirst (row: row.flushAccount == account) null tpmPrincipals).flushUid;
  }));

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
    (lib.genAttrs (map (row: row.account) tpmPrincipals) (account: {
      isSystemUser = true;
      group = account;
      uid = (lib.findFirst (row: row.account == account) null tpmPrincipals).ownerUid;
      description = "d2b Device TPM state owner";
    }))
    (lib.genAttrs (map (row: row.flushAccount) tpmPrincipals) (account: {
      isSystemUser = true;
      group = account;
      uid = (lib.findFirst (row: row.flushAccount == account) null tpmPrincipals).flushUid;
      description = "d2b Device TPM pre-start flush principal";
    }))
  ];
}
