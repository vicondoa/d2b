# Private configured argv and shell policy for unsafe-local workloads.
{ config, lib, ... }:

let
  cfg = config.d2b;
  unsafeLocalWorkloads = lib.filter
    (workload: workload.kind == "unsafe-local")
    cfg._index.realms.workloads.enabled;
  localVmWorkloads = lib.filter
    (workload: workload.kind == "local-vm")
    cfg._index.realms.workloads.enabled;

  privateItem = item:
    if item.type == "exec"
    then {
      type = "exec";
      inherit (item) id name argv graphical;
      icon = lib.filterAttrs (_: value: value != null) item.icon;
    }
    else {
      type = "shell";
      inherit (item) id name;
      icon = lib.filterAttrs (_: value: value != null) item.icon;
    };

  privateWorkload = workload:
    lib.filterAttrs (_: value: value != null) {
      identity = lib.filterAttrs (_: value: value != null) {
        workloadId = workload.workloadId;
        workloadName =
          if workload.label == workload.workloadId
          then null
          else workload.label;
        realmId = workload.realmId;
        realmPath = lib.splitString "." workload.realmPath;
        canonicalTarget = workload.canonicalTarget;
        legacyVmName = null;
        runtimeKind = "unsafe-local";
        providerId = "unsafe-local";
      };
      defaultItemId = workload.defaultItemId;
      items = map privateItem workload.launcherItems;
      shell =
        if workload.shell.enable
        then {
          inherit (workload.shell) defaultName maxSessions;
        }
        else null;
    };

  privateLocalVmWorkload = workload: {
    identity = lib.filterAttrs (_: value: value != null) {
      workloadId = workload.workloadId;
      workloadName =
        if workload.label == workload.workloadId
        then null
        else workload.label;
      realmId = workload.realmId;
      realmPath = lib.splitString "." workload.realmPath;
      canonicalTarget = workload.canonicalTarget;
      legacyVmName = workload.legacyVmName;
      runtimeKind = workload.runtimeKind;
      providerId = workload.runtimeProviderId;
    };
    defaultItemId = workload.defaultItemId;
    items = map privateItem workload.launcherItems;
  };

  data = {
    schemaVersion = "v2";
    workloads = map privateWorkload unsafeLocalWorkloads;
    localVmWorkloads = map privateLocalVmWorkload localVmWorkloads;
  };
in
{
  config.d2b._bundle.unsafeLocalWorkloadsJson = {
    inherit data;
    installFileName = "unsafe-local-workloads.json";
    classification = "contractPrivateNonSecret";
    sensitivity = "nonSecret";
  };
}
