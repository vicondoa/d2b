{ config ? null
, cfg ? null
, lib
, identity ? import ../v2-identity.nix
, generation ? 1
}:

let
  effectiveCfg =
    if cfg != null then cfg
    else if config != null then config.d2b
    else null;
  implementations = [ "wayland" ];
  capabilities = [
    "display.open"
    "display.inspect"
    "display.adopt"
    "display.close"
  ];

  validateOpaqueId = value:
    if builtins.isString value
      && builtins.stringLength value <= 64
      && builtins.match "[a-z][a-z0-9-]*" value != null
    then value
    else throw "provider display mapping: invalid opaque generated id";

  validateControllerRole = value:
    if builtins.elem value [ "local-root-controller" "realm-controller" ]
    then value
    else throw "provider display mapping: invalid in-process controller placement";

  mkEntry = mapping:
    let
      providerId = identity.validateShortId mapping.providerId;
      realmId = identity.validateShortId mapping.realmId;
      workloadId = identity.validateShortId mapping.workloadId;
      ownerRoleId = identity.validateShortId mapping.ownerRoleId;
      canonicalProviderId = identity.deriveProviderId
        realmId "display" "wayland-${workloadId}";
      canonicalOwnerRoleId = identity.deriveRoleId
        realmId workloadId "wayland-proxy";
      implementationId =
        if mapping.implementationId == "wayland"
        then mapping.implementationId
        else throw "provider display mapping: unregistered display implementation";
      controllerRole = validateControllerRole mapping.controllerRole;
      endpointIds = {
        wayland = validateOpaqueId mapping.endpointIds.wayland;
        crossDomain = validateOpaqueId mapping.endpointIds.crossDomain;
        waypipe = validateOpaqueId mapping.endpointIds.waypipe;
        proxy = validateOpaqueId mapping.endpointIds.proxy;
      };
      binding =
        if builtins.length (lib.unique (lib.attrValues endpointIds)) != 4 then
          throw "provider display mapping: generated endpoint ids must be distinct"
        else {
          axis = "local-display";
          inherit workloadId ownerRoleId endpointIds;
        };
      configurationSchemaFingerprint = builtins.hashString "sha256"
        "d2b-provider-display-wayland-configuration-v1";
      configuredScopeDigest = builtins.hashString "sha256" (builtins.toJSON {
        inherit providerId realmId implementationId controllerRole binding;
      });
    in
    if providerId != canonicalProviderId then
      throw "provider display mapping: provider id is not workload-scoped"
    else if ownerRoleId != canonicalOwnerRoleId then
      throw "provider display mapping: owner role is not the canonical Wayland role"
    else builtins.deepSeq
      [ providerId realmId workloadId ownerRoleId implementationId controllerRole binding ]
      {
        descriptor = {
          schemaVersion = 2;
          inherit providerId implementationId;
          authority.type = "display";
          apiVersion = {
            major = 2;
            minor = 0;
          };
          inherit capabilities configurationSchemaFingerprint configuredScopeDigest;
          registryGeneration = generation;
          placement = {
            kind = "trusted-first-party-in-process";
            inherit realmId controllerRole;
          };
        };
        inherit binding;
      };

  mkEntries = mappings:
    let
      entries = map mkEntry mappings;
      providerIds = map (entry: entry.descriptor.providerId) entries;
    in
    if builtins.length providerIds != builtins.length (lib.unique providerIds) then
      throw "provider display mapping: duplicate provider id"
    else
      lib.sort
        (left: right:
          lib.lessThan left.descriptor.providerId right.descriptor.providerId)
        entries;

  configuredMappings =
    if effectiveCfg == null then [ ]
    else if !(effectiveCfg._index ? providerRegistryV2Mappings) then
      throw "provider display mapping: authoritative normalized mapping seam is missing"
    else effectiveCfg._index.providerRegistryV2Mappings.display;
  providers = mkEntries configuredMappings;
in
{
  inherit implementations mkEntries providers;
}
