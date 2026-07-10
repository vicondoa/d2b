# Provider-neutral, argv-free desktop launcher metadata.
{ config, lib, ... }:

let
  cfg = config.d2b;
  workloads = cfg._index.realms.workloads.enabled;

  publicItem = item: {
    inherit (item) id type name graphical;
    icon = lib.filterAttrs (_: value: value != null) item.icon;
    capabilities = item.capabilityRefs;
  };

  publicWorkload = workload: {
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
    providerKind = workload.providerKind;
    executionPosture = workload.executionPosture;
    label = workload.label;
    icon = lib.filterAttrs (_: value: value != null) {
      id = workload.iconId;
      name = workload.iconName;
    };
    realmAccentColor = cfg._uiColors.realms.${workload.realmName}.accent;
    launcherEnabled = workload.launcherEnabled;
    defaultItemId = workload.defaultItemId;
    capabilities = workload.capabilityRefs;
    items = map publicItem workload.launcherItems;
  };

  data = {
    schemaVersion = "v2";
    runtimeState = "contract-only";
    workloads = map publicWorkload workloads;
    invariants = {
      argvPrivate = true;
      providerNeutral = true;
      typedExecutionPosture = true;
      realmAccentColorOnly = true;
      noSecretsOrCredentials = true;
    };
  };
in
{
  config.d2b._bundle.realmWorkloadsLauncherV2Json = {
    inherit data;
    installFileName = "realm-workloads-launcher-v2.json";
    classification = "contractPublic";
    sensitivity = "nonSecret";
  };
}
