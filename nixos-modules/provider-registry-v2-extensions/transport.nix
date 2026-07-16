{ lib
, identity ? import ../v2-identity.nix
, generation ? 1
}:

let
  implementations = [
    "cloud-hypervisor-vsock"
    "native-vsock"
    "unix-seqpacket"
    "unix-stream"
  ];
  capabilities = [
    "transport.connect"
    "transport.revoke-binding"
    "transport.inspect"
  ];

  validateOpaqueId = value:
    if builtins.isString value
      && builtins.stringLength value <= 64
      && builtins.match "[a-z][a-z0-9-]*" value != null
    then value
    else throw "provider transport mapping: invalid opaque generated id";

  validateControllerRole = value:
    if builtins.elem value [ "local-root-controller" "realm-controller" ]
    then value
    else throw "provider transport mapping: invalid in-process controller placement";

  mkEntry = mapping:
    let
      providerId = identity.validateShortId mapping.providerId;
      realmId = identity.validateShortId mapping.realmId;
      implementationId =
        if builtins.elem mapping.implementationId implementations
        then mapping.implementationId
        else throw "provider transport mapping: unregistered local implementation";
      controllerRole = validateControllerRole mapping.controllerRole;
      transportBindingIds = lib.sort lib.lessThan
        (map validateOpaqueId mapping.transportBindingIds);
      bindingIdsAreUnique =
        builtins.length transportBindingIds
        == builtins.length (lib.unique transportBindingIds);
      binding =
        if transportBindingIds == [ ] then
          throw "provider transport mapping: at least one generated binding id is required"
        else if !bindingIdsAreUnique then
          throw "provider transport mapping: duplicate generated binding id"
        else {
          axis = "local-transport";
          inherit transportBindingIds;
        };
      configurationSchemaFingerprint = builtins.hashString "sha256"
        "d2b-provider-transport-${implementationId}-configuration-v1";
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
          authority.type = "transport";
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
      uniqueProviderIds = lib.unique providerIds;
    in
    if builtins.length providerIds != builtins.length uniqueProviderIds then
      throw "provider transport mapping: duplicate provider id"
    else
      lib.sort
        (left: right:
          lib.lessThan left.descriptor.providerId right.descriptor.providerId)
        entries;
in
{
  inherit implementations mkEntries;
}
