# Private configured launcher intents keyed by canonical workload/provider identity.
{ config, lib, ... }:

let
  cfg = config.d2b;
  workloads = cfg._index.workloads.enabledList;

  runtimeBinding = workload:
    workload.providerBindings.runtime or
      (throw "private launcher intent: workload ${workload.canonicalTarget} has no runtime provider binding");

  hasConfiguredLaunch = workload:
    workload.launcher.enabled
    && (workload.launcher.items != { }
      || workload.launcher.defaultItem != null
      || (workload.spec.shell.enable or false));

  configuredWorkloads = lib.filter hasConfiguredLaunch workloads;
  unsafeLocalWorkloads = lib.filter
    (workload: (runtimeBinding workload).implementationId == "systemd-user")
    configuredWorkloads;
  localVmWorkloads = lib.filter
    (workload:
      builtins.elem (runtimeBinding workload).implementationId
        [ "cloud-hypervisor" "qemu-media" ])
    configuredWorkloads;

  privateItem = itemId: item:
    if item.type == "exec"
    then {
      type = "exec";
      id = itemId;
      inherit (item) name argv graphical;
      icon = lib.filterAttrs (_: value: value != null) item.icon;
    }
    else if item.type == "shell"
    then {
      type = "shell";
      id = itemId;
      inherit (item) name;
      icon = lib.filterAttrs (_: value: value != null) item.icon;
    }
    else null;

  privateItems = items:
    lib.filter (item: item != null) (lib.mapAttrsToList privateItem items);

  privateIdentity = workload:
    let runtime = runtimeBinding workload;
    in {
      inherit (workload) workloadId realmId canonicalTarget;
      realmPath = lib.splitString "." workload.realmPath;
      runtimeKind = runtime.implementationId;
      providerId = runtime.providerId;
    };

  privateWorkload = workload:
    lib.filterAttrs (_: value: value != null) {
      identity = privateIdentity workload;
      defaultItemId = workload.launcher.defaultItem;
      items = privateItems workload.launcher.items;
      shell =
        if workload.spec.shell.enable or false
        then {
          inherit (workload.spec.shell) defaultName maxSessions;
        }
        else null;
    };

  privateLocalVmWorkload = workload:
    lib.filterAttrs (_: value: value != null) {
      identity = privateIdentity workload;
      defaultItemId = workload.launcher.defaultItem;
      items = privateItems workload.launcher.items;
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
