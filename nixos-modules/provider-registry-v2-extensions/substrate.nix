{ lib
, identity ? import ../v2-identity.nix
, generation ? 1
}:

let
  implementations = [ "linux" "nixos" ];
  capabilities = [
    "substrate.check"
    "substrate.plan-remediation"
    "substrate.apply"
  ];

  mkEntry = mapping:
    let
      providerId = identity.validateShortId mapping.providerId;
      realmId = identity.validateShortId mapping.realmId;
      implementationId =
        if builtins.elem mapping.implementationId implementations
        then mapping.implementationId
        else throw "provider substrate mapping: unregistered host implementation";
      binding = {
        axis = "local-substrate";
      };
      configurationSchemaFingerprint = builtins.hashString "sha256"
        "d2b-provider-substrate-${implementationId}-configuration-v1";
      configuredScopeDigest = builtins.hashString "sha256" (builtins.toJSON {
        inherit providerId realmId implementationId binding;
        controllerRole = "local-root-controller";
      });
    in
    builtins.deepSeq [ providerId realmId implementationId ] {
      descriptor = {
        schemaVersion = 2;
        inherit providerId implementationId;
        authority.type = "substrate";
        apiVersion = {
          major = 2;
          minor = 0;
        };
        inherit capabilities configurationSchemaFingerprint configuredScopeDigest;
        registryGeneration = generation;
        placement = {
          kind = "trusted-first-party-in-process";
          inherit realmId;
          controllerRole = "local-root-controller";
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
      throw "provider substrate mapping: duplicate provider id"
    else
      lib.sort
        (left: right:
          lib.lessThan left.descriptor.providerId right.descriptor.providerId)
        entries;
in
{
  inherit implementations mkEntries;
}
