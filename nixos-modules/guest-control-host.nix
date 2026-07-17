{ config, lib, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  workloadRows = import ./workload-process-rows.nix {
    inherit config lib;
  };
  keyResourceFor = row:
    lib.findFirst
      (resource: resource.kind == "workload-keys")
      (throw "workload ${row.workloadId} is missing its key resource")
      (cfg._index.resources.byWorkloadId.${row.workloadId} or [ ]);
  rows = map
    (row:
      let
        role = lib.findFirst
          (candidate: candidate.roleKind == "virtiofsd")
          (throw "workload ${row.workloadId} is missing its virtiofsd role")
          row.roles;
      in
      {
        inherit (row) realmId workloadId canonicalTarget;
        roleId = role.roleId;
        resourceRef = (keyResourceFor row).resourceId;
        target = "${(keyResourceFor row).path}/guest-control/token";
        readerUid = d2bLib.stablePrincipalId
          "d2b-gctlfs-${row.workloadId}";
        readerGid = d2bLib.stablePrincipalId
          "d2b-gctlfs-${row.workloadId}";
        mode = "0440";
        source = "generated";
        creator = "realm-broker";
        repairOwner = "realm-broker";
        materializedByHostActivation = false;
      })
    (lib.filter
      (row: row.runtimeImplementation == "cloud-hypervisor")
      workloadRows);
in
{
  options.d2b._workloadGuestControlRows = lib.mkOption {
    type = lib.types.listOf lib.types.attrs;
    internal = true;
    visible = false;
    readOnly = true;
  };

  config.d2b._workloadGuestControlRows = rows;
}
