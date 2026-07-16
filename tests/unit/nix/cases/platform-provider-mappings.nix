{ lib, flakeRoot, ... }:

let
  identity = import "${flakeRoot}/nixos-modules/v2-identity.nix";
  transport = import
    "${flakeRoot}/nixos-modules/provider-registry-v2-extensions/transport.nix"
    { inherit lib identity; generation = 7; };
  substrate = import
    "${flakeRoot}/nixos-modules/provider-registry-v2-extensions/substrate.nix"
    { inherit lib identity; generation = 7; };
  display = import
    "${flakeRoot}/nixos-modules/provider-registry-v2-extensions/display.nix"
    { inherit lib identity; generation = 7; };

  realmId = identity.deriveRealmId "work.local-root";
  localRootRealmId = identity.deriveRealmId "local-root";
  workloadId = identity.deriveWorkloadId realmId "editor";
  ownerRoleId = identity.deriveRoleId realmId workloadId "wayland-proxy";
  providerId = providerType: configuredId:
    identity.deriveProviderId realmId providerType configuredId;
  localRootProviderId = configuredId:
    identity.deriveProviderId localRootRealmId "substrate" configuredId;

  transportEntries = transport.mkEntries [
    {
      providerId = providerId "transport" "unix-stream";
      inherit realmId;
      implementationId = "unix-stream";
      controllerRole = "realm-controller";
      transportBindingIds = [ "binding-work-public" "binding-work-broker" ];
    }
    {
      providerId = providerId "transport" "native-vsock";
      inherit realmId;
      implementationId = "native-vsock";
      controllerRole = "realm-controller";
      transportBindingIds = [ "binding-editor-guest" ];
    }
  ];

  substrateEntries = substrate.mkEntries [
    {
      providerId = localRootProviderId "nixos";
      realmId = localRootRealmId;
      implementationId = "nixos";
    }
    {
      providerId = localRootProviderId "linux";
      realmId = localRootRealmId;
      implementationId = "linux";
    }
  ];

  displayEntries = display.mkEntries [{
    providerId = providerId "display" "wayland-editor";
    inherit realmId workloadId ownerRoleId;
    implementationId = "wayland";
    controllerRole = "realm-controller";
    endpointIds = {
      wayland = "endpoint-wayland";
      crossDomain = "endpoint-cross-domain";
      waypipe = "endpoint-waypipe";
      proxy = "endpoint-proxy";
    };
  }];

  project = entry: {
    inherit (entry.descriptor)
      authority
      implementationId
      capabilities
      registryGeneration
      placement
      ;
    providerIdLength = builtins.stringLength entry.descriptor.providerId;
    schemaFingerprintLength =
      builtins.stringLength entry.descriptor.configurationSchemaFingerprint;
    scopeDigestLength =
      builtins.stringLength entry.descriptor.configuredScopeDigest;
    inherit (entry) binding;
  };
in
{
  "platform-provider-mappings/transport-closed-and-canonical" = {
    expr = {
      implementations = transport.implementations;
      sorted = lib.lessThan
        (builtins.elemAt transportEntries 0).descriptor.providerId
        (builtins.elemAt transportEntries 1).descriptor.providerId;
    };
    expected = {
      implementations = [
        "cloud-hypervisor-vsock"
        "native-vsock"
        "unix-seqpacket"
        "unix-stream"
      ];
      sorted = true;
    };
  };

  "platform-provider-mappings/transport-contract-shape" = {
    expr = project (lib.findFirst
      (entry: entry.descriptor.implementationId == "unix-stream")
      null
      transportEntries);
    expected = {
      authority.type = "transport";
      implementationId = "unix-stream";
      capabilities = [
        "transport.connect"
        "transport.revoke-binding"
        "transport.inspect"
      ];
      registryGeneration = 7;
      placement = {
        kind = "trusted-first-party-in-process";
        inherit realmId;
        controllerRole = "realm-controller";
      };
      providerIdLength = 20;
      schemaFingerprintLength = 64;
      scopeDigestLength = 64;
      binding = {
        axis = "local-transport";
        transportBindingIds = [ "binding-work-broker" "binding-work-public" ];
      };
    };
  };

  "platform-provider-mappings/substrate-local-root-placement" = {
    expr = project (lib.findFirst
      (entry: entry.descriptor.implementationId == "nixos")
      null
      substrateEntries);
    expected = {
      authority.type = "substrate";
      implementationId = "nixos";
      capabilities = [
        "substrate.check"
        "substrate.plan-remediation"
        "substrate.apply"
      ];
      registryGeneration = 7;
      placement = {
        kind = "trusted-first-party-in-process";
        realmId = localRootRealmId;
        controllerRole = "local-root-controller";
      };
      providerIdLength = 20;
      schemaFingerprintLength = 64;
      scopeDigestLength = 64;
      binding.axis = "local-substrate";
    };
  };

  "platform-provider-mappings/display-opaque-binding" = {
    expr = project (builtins.head displayEntries);
    expected = {
      authority.type = "display";
      implementationId = "wayland";
      capabilities = [
        "display.open"
        "display.inspect"
        "display.adopt"
        "display.close"
      ];
      registryGeneration = 7;
      placement = {
        kind = "trusted-first-party-in-process";
        inherit realmId;
        controllerRole = "realm-controller";
      };
      providerIdLength = 20;
      schemaFingerprintLength = 64;
      scopeDigestLength = 64;
      binding = {
        axis = "local-display";
        inherit workloadId ownerRoleId;
        endpointIds = {
          wayland = "endpoint-wayland";
          crossDomain = "endpoint-cross-domain";
          waypipe = "endpoint-waypipe";
          proxy = "endpoint-proxy";
        };
      };
    };
  };

  "platform-provider-mappings/reject-unregistered-transport" = {
    expr = transport.mkEntries [{
      providerId = providerId "transport" "loopback";
      inherit realmId;
      implementationId = "loopback";
      controllerRole = "realm-controller";
      transportBindingIds = [ "binding-test" ];
    }];
    expectedError = { };
  };

  "platform-provider-mappings/reject-human-display-id" = {
    expr = display.mkEntries [{
      providerId = "wayland-editor";
      inherit realmId workloadId ownerRoleId;
      implementationId = "wayland";
      controllerRole = "realm-controller";
      endpointIds = {
        wayland = "endpoint-wayland";
        crossDomain = "endpoint-cross-domain";
        waypipe = "endpoint-waypipe";
        proxy = "endpoint-proxy";
      };
    }];
    expectedError = { };
  };

  "platform-provider-mappings/reject-display-endpoint-alias" = {
    expr = display.mkEntries [{
      providerId = providerId "display" "wayland-editor";
      inherit realmId workloadId ownerRoleId;
      implementationId = "wayland";
      controllerRole = "realm-controller";
      endpointIds = {
        wayland = "endpoint-wayland";
        crossDomain = "endpoint-wayland";
        waypipe = "endpoint-waypipe";
        proxy = "endpoint-proxy";
      };
    }];
    expectedError = { };
  };
}
