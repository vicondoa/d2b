# nixos-modules/realm-workloads-launcher-json.nix
#
# Generates realm-workloads-launcher.json — stable desktop launcher metadata
# consumed by Waybar / wlcontrol / wlterm / clip-picker and similar host-side
# tooling that needs to discover and launch realm-owned workloads.
#
# Security contract:
#   • No secrets, provider tokens, opaque session handles, or sensitive
#     payloads.  Every field is either static operator-declared metadata, a
#     stable opaque ref, or a non-secret descriptor.
#   • appCommand and actions[].command are static launch commands declared
#     by the operator in Nix.  They do not contain credentials, tokens, or
#     dynamic runtime data.  Consumers must not treat them as trusted inputs
#     and should validate them before shell-expansion.
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

  # Derive vsockCid for a workload whose legacyVmName points to a local
  # NixOS VM.  Null for non-nixos, missing, or no legacyVmName — callers
  # must handle null.
  vsockCidFor = legacyVmName:
    if legacyVmName == null then null
    else if !(builtins.hasAttr legacyVmName enabledVms) then null
    else if d2bLib.vmRuntimeKind enabledVms.${legacyVmName} != "nixos" then null
    else cfg.manifest.${legacyVmName}.observability.vsockCid;

  # Build a single launcher metadata row from a workload index entry.
  launcherRow = workload: {
    realmName = workload.realmName;
    realmId = workload.realmId;
    realmPath = workload.realmPath;
    workloadName = workload.workloadName;
    targetAddress = workload.targetAddress;
    # canonicalTarget: routing address for realm-native desktop tooling.
    # Equals targetAddress unless launcher.app.targetRealm overrides it.
    canonicalTarget = workload.canonicalTarget;
    kind = workload.kind;
    actionId = workload.actionId;
    label = workload.label;
    icon = workload.icon;
    capabilityRefs = workload.capabilityRefs;
    # appCommand: static operator-declared primary launch command; null when
    # not set.  Consumers must not shell-expand without validation.
    appCommand = workload.appCommand;
    # actions: additional named launcher actions with id, label, command.
    # Commands are static operator-declared metadata, not sensitive payloads.
    actions = workload.actions;
    legacyVmName = workload.legacyVmName;
    substrateId = workload.substrateId;
    runtimeKind = workload.runtimeKind;
    runtimeProviderId = workload.runtimeProviderId;
    # vsockCid: advisory hint for vsock-based workload readiness checks.
    # Null when legacyVmName is absent, non-nixos, or disabled.  Must not
    # be used as an authentication token.
    vsockCid = vsockCidFor workload.legacyVmName;
  };

  launcherRows = map launcherRow realmWorkloads;

  data = {
    schemaVersion = "v1";
    runtimeState = "metadata-only";
    workloads = launcherRows;
    invariants = {
      # Affirm that no secrets, credentials, or sensitive payloads appear.
      noSecretsOrCredentials = true;
      # appCommand and actions[].command are static operator-declared launch
      # metadata — not sensitive command payloads or dynamic runtime data.
      noSensitiveCommandPayloads = true;
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
