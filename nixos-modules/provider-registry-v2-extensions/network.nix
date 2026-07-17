{ config
, lib
, generation ? 1
, identity ? import ../v2-identity.nix
, ...
}:

let
  plan = import ../realm-network-rows.nix {
    inherit config lib;
  };
  capabilities = [
    "network.plan"
    "network.ensure"
    "network.inspect"
    "network.adopt"
    "network.release"
  ];
  configurationSchemaFingerprint =
    builtins.hashString "sha256"
      "d2b-provider-network-local-realm-configuration-v1";

  provider = realm:
    let
      configuredProviderId = "network-local";
      providerId = identity.deriveProviderId
        realm.canonicalRealmId "network" configuredProviderId;
      binding = {
        axis = "network";
        inherit (realm.providerBinding)
          networkId
          allocatorLeaseId
          bridgeSetId
          tapSetId
          netVmRoleId
          natPolicyId
          dhcpPolicyId
          nftPolicyId
          netlinkPolicyId
          externalAttachmentId
          resourceGeneration
          ;
      };
    in
    {
      descriptor = {
        schemaVersion = 2;
        inherit providerId;
        authority.type = "network";
        implementationId = "local-realm";
        apiVersion = {
          major = 2;
          minor = 0;
        };
        inherit capabilities configurationSchemaFingerprint;
        configuredScopeDigest =
          builtins.hashString "sha256" (builtins.toJSON {
            inherit providerId binding;
          });
        registryGeneration = generation;
        placement = {
          kind = "trusted-first-party-in-process";
          realmId = realm.canonicalRealmId;
          controllerRole = "realm-controller";
        };
      };
      inherit binding;
    };
in
{
  axis = "network";
  inherit generation;
  providers = map provider plan.realms;
}
