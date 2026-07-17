{ config, lib, ... }:

let
  cfg = config.d2b;
  realmStorageRows = import ./realm-storage-rows.nix { inherit config lib; };
  storeLiveRows = lib.filter
    (row: lib.hasSuffix "/store-view-live" row.id)
    realmStorageRows.paths;
  hasInvariant = invariant: row: builtins.elem invariant row.invariants;
  hard_linkFarmRoot = toString cfg.store.stateDir;
in
{
  options.d2b.store = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Materialise a closure-only hardlink store view for each local VM
        workload. The broker is the sole creator, synchronizer, and repair
        owner; Nix activation never mutates a store-view tree.
      '';
    };

    stateDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/d2b/vms";
      description = ''
        Compatibility root for code that has not yet consumed realm storage
        rows. Realm-native store views always use generated short-ID paths
        below `/var/lib/d2b/r`. Every hardlink farm must remain on the same
        filesystem as `/nix/store`.
      '';
    };
  };

  config.assertions = lib.optionals cfg.store.enable [
    {
      assertion = lib.all
        (row:
          row.recursive == false
          && hasInvariant "same-filesystem" row
          && hasInvariant "hardlink-farm-no-recursion" row
          && hasInvariant "no-recursive-mutation" row
          && row.creator.kind == "broker"
          && row.repairPolicy == "broker-reconcile")
        storeLiveRows;
      message = ''
        Realm workload store-view live rows must stay broker-owned,
        same-filesystem, and non-recursive so hardlink operations cannot
        propagate ownership or ACL changes into /nix/store.
      '';
    }
    {
      assertion = hard_linkFarmRoot != "/nix/store";
      message = ''
        The workload hardlink-farm root must not be /nix/store; store views
        expose only the workload closure and are populated by the broker.
      '';
    }
  ];
}
