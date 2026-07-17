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
  implementations = [ "host-mediated" ];
  capabilities = [
    "device.plan-attach"
    "device.attach"
    "device.inspect"
    "device.adopt"
    "device.detach"
  ];

  validateControllerRole = value:
    if builtins.elem value [ "local-root-controller" "realm-controller" ]
    then value
    else throw "provider device mapping: invalid in-process controller placement";

  mkEntry = mapping:
    let
      providerId = identity.validateShortId mapping.providerId;
      realmId = identity.validateShortId mapping.realmId;
      implementationId =
        if builtins.elem mapping.implementationId implementations
        then mapping.implementationId
        else throw "provider device mapping: unregistered host implementation";
      controllerRole = validateControllerRole mapping.controllerRole;
      deviceResourceIds = lib.sort lib.lessThan
        (lib.unique mapping.deviceResourceIds);
      binding = {
        axis = "local-device";
        inherit deviceResourceIds;
      };
      configurationSchemaFingerprint = builtins.hashString "sha256"
        "d2b-provider-device-host-mediated-configuration-v1";
      configuredScopeDigest = builtins.hashString "sha256" (builtins.toJSON {
        inherit providerId realmId implementationId controllerRole binding;
      });
    in
    builtins.deepSeq
      [ providerId realmId implementationId controllerRole binding ]
      {
        descriptor = {
          schemaVersion = 2;
          inherit providerId implementationId;
          authority.type = "device";
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
      throw "provider device mapping: duplicate provider id"
    else
      lib.sort
        (left: right:
          lib.lessThan left.descriptor.providerId right.descriptor.providerId)
        entries;

  index =
    if effectiveCfg == null then { }
    else (effectiveCfg._index or { });
  realmRows = index.realms.enabledList or [ ];
  realmsById = lib.listToAttrs (map
    (realm: {
      name = realm.realmId;
      value = realm;
    })
    realmRows);
  providerRows = index.providers.enabledList or [ ];
  deviceRows = index.devices.list or [ ];
  configuredMappings = map
    (provider:
      let realm = realmsById.${provider.realmId};
      in {
        inherit (provider) providerId realmId implementationId;
        controllerRole =
          if (realm.parentRealmId or null) == null
            && (realm.parentPath or null) == null
          then "local-root-controller"
          else "realm-controller";
        deviceResourceIds = map
          (row: row.resourceId)
          (lib.filter
            (row: row.providerId == provider.providerId)
            deviceRows);
      })
    (lib.filter
      (provider:
        provider.providerType == "device"
        && builtins.elem provider.implementationId implementations
        && builtins.hasAttr provider.realmId realmsById
        && (realmsById.${provider.realmId}.placement or "host-local")
          == "host-local")
      providerRows);
  providers = mkEntries configuredMappings;
in
{
  inherit implementations mkEntries providers;
}
