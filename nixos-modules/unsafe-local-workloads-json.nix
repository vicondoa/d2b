# Private configured argv and shell policy for unsafe-local workloads.
{ config, lib, ... }:

let
  cfg = config.d2b;
  unsafeLocalWorkloads = lib.filter
    (workload: workload.kind == "unsafe-local")
    cfg._index.realms.workloads.enabled;
  hasConfiguredLocalVmLaunch = workload:
    let
      declared =
        cfg.realms.${workload.realmName}.workloads.${workload.workloadName};
    in
    workload.kind == "local-vm"
    && workload.launcherEnabled
    && (declared.launcher.items != { }
      || declared.launcher.defaultItem != null
      || declared.shell.enable);
  localVmWorkloads = lib.filter
    hasConfiguredLocalVmLaunch
    cfg._index.realms.workloads.enabled;

  privateItem = item:
    if item.type == "exec"
    then {
      type = "exec";
      inherit (item) id name argv graphical;
      icon = lib.filterAttrs (_: value: value != null) item.icon;
    }
    else if item.type == "shell"
    then {
      type = "shell";
      inherit (item) id name;
      icon = lib.filterAttrs (_: value: value != null) item.icon;
    }
    else null;

  privateItems = items:
    lib.filter (item: item != null) (map privateItem items);

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
      items = privateItems workload.launcherItems;
      shell =
        if workload.shell.enable
        then {
          inherit (workload.shell) defaultName maxSessions;
        }
        else null;
    };

  privateLocalVmWorkload = workload:
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
        legacyVmName = workload.legacyVmName;
        runtimeKind = workload.runtimeKind;
        providerId = workload.runtimeProviderId;
      };
      defaultItemId = workload.defaultItemId;
      items = privateItems workload.launcherItems;
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
