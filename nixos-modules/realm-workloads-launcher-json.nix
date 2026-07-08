# nixos-modules/realm-workloads-launcher-json.nix
#
# Generates realm-workloads-launcher.json — stable desktop launcher metadata
# consumed by Waybar / wlcontrol / wlterm / clip-picker and similar host-side
# tooling that needs to discover and launch realm-owned workloads.
#
# Security contract:
#   • No secrets, provider tokens, opaque session handles, or sensitive
#     command payloads. Every field is either static metadata, a stable
#     opaque ref, or a non-secret descriptor.
#   • vsockCid is included as a numeric advisory hint only (it is never
#     an authentication token); launchers that read it MUST treat it as
#     informational and MUST NOT use it as an authorization factor.
#   • The artifact is installed at root:d2bd 0640 (contractPrivateNonSecret)
#     so unprivileged desktop processes cannot read it directly; launchers
#     must obtain it through the d2bd public socket or a helper with the
#     right ACL.
{ config, lib, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };

  sortNames = names: lib.sort lib.lessThan names;
  sortedAttrNames = attrs: sortNames (lib.attrNames attrs);
  sortedMapAttrsToList = f: attrs:
    map (name: f name attrs.${name}) (sortedAttrNames attrs);

  enabledVms = cfg._index.enabledVms;
  realmWorkloads = cfg._index.realms.workloads.enabled;

  # Derive vsockCid for a vmRef pointing to a normalNixos VM.
  # Null for non-nixos or missing VMs — callers must handle null.
  vsockCidFor = vmRef:
    if vmRef == null then null
    else if !(builtins.hasAttr vmRef enabledVms) then null
    else if d2bLib.vmRuntimeKind enabledVms.${vmRef} != "nixos" then null
    else cfg.manifest.${vmRef}.observability.vsockCid;

  # Build a single launcher metadata row from a workload index entry.
  launcherRow = workload: {
    realmName = workload.realmName;
    realmId = workload.realmId;
    realmPath = workload.realmPath;
    workloadName = workload.workloadName;
    targetAddress = workload.targetAddress;
    actionId = workload.actionId;
    label = workload.label;
    icon = workload.icon;
    capabilityRefs = workload.capabilityRefs;
    preflightRefs = workload.preflightRefs;
    vmRef = workload.vmRef;
    substrateId = workload.substrateId;
    runtimeKind = workload.runtimeKind;
    runtimeProviderId = workload.runtimeProviderId;
    # vsockCid: advisory hint for vsock-based workload readiness checks.
    # Null when vmRef is absent, non-nixos, or disabled. Must not be used
    # as an authentication token.
    vsockCid = vsockCidFor workload.vmRef;
  };

  launcherRows = map launcherRow realmWorkloads;

  data = {
    schemaVersion = "v1";
    runtimeState = "metadata-only";
    workloads = launcherRows;
    invariants = {
      # Affirm that no secrets, credentials, or sensitive payloads appear.
      noSecretsOrCredentials = true;
      noCommandPayloads = true;
      noOpaqueSessionHandles = true;
      noProviderTokens = true;
      metadataOnly = true;
    };
  };
in
{
  config.d2b._bundle.realmWorkloadsLauncherJson = {
    inherit data;
    installFileName = "realm-workloads-launcher.json";
    classification = "contractPrivateNonSecret";
    sensitivity = "nonSecret";
  };
}
