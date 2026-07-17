{ config, lib, pkgs, generation ? 1, ... }:

let
  identity = import ../v2-identity.nix;
  rows = import ../realm-audio-rows.nix { inherit config lib pkgs; };
  capabilities = [
    "audio.open"
    "audio.set-state"
    "audio.inspect"
    "audio.adopt"
    "audio.close"
  ];

  provider = workload:
    let
      providerId = identity.deriveProviderId
        workload.realmId "audio" "audio-${workload.workloadId}";
      binding = {
        axis = "local-audio";
        inherit (workload)
          workloadId
          roleId
          processId
          endpointId
          stateStorageId
          lockStorageId
          mediationStorageId
          leaseId
          ;
      };
    in
    {
      descriptor = {
        schemaVersion = 2;
        inherit providerId;
        authority.type = "audio";
        implementationId = "pipewire-vhost-user";
        apiVersion = {
          major = 2;
          minor = 0;
        };
        inherit capabilities;
        configurationSchemaFingerprint = builtins.hashString "sha256"
          "d2b-provider-audio-pipewire-vhost-user-configuration-v1";
        configuredScopeDigest = builtins.hashString "sha256"
          (builtins.toJSON {
            inherit providerId binding;
          });
        registryGeneration = generation;
        placement = {
          kind = "trusted-first-party-in-process";
          realmId = workload.realmId;
          controllerRole = "realm-controller";
        };
      };
      inherit binding;
    };
in
{
  axis = "audio";
  inherit generation;
  providers = lib.sort
    (left: right:
      lib.lessThan left.descriptor.providerId right.descriptor.providerId)
    (map provider rows.workloads);
}
