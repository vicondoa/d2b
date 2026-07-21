{ config, lib, ... }:

let
  cfg = config.d2b;
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
      let root = "${(keyResourceFor row).path}/sshd";
      in {
        inherit (row) realmId workloadId canonicalTarget;
        resourceRef = (keyResourceFor row).resourceId;
        privateKey = "${root}/ssh_host_ed25519_key";
        publicKey = "${root}/ssh_host_ed25519_key.pub";
        algorithm = "ed25519";
        privateMode = "0400";
        publicMode = "0644";
        creator = "realm-broker";
        repairOwner = "realm-broker";
        materializedByHostActivation = false;
      })
    (lib.filter
      (row: row.runtimeImplementation == "cloud-hypervisor")
      workloadRows);
in
{
  options.d2b._workloadSshHostKeyRows = lib.mkOption {
    type = lib.types.listOf lib.types.attrs;
    internal = true;
    visible = false;
    readOnly = true;
  };

  config.d2b._workloadSshHostKeyRows = rows;
}
