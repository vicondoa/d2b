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
  unixStreamProviderId = providerId "transport" "unix-stream";
  nativeVsockProviderId = providerId "transport" "native-vsock";
  nixosProviderId = localRootProviderId "nixos";
  linuxProviderId = localRootProviderId "linux";
  waylandProviderId = providerId "display" "wayland-${workloadId}";

  transportEntries = transport.mkEntries [
    {
      providerId = unixStreamProviderId;
      inherit realmId;
      implementationId = "unix-stream";
      controllerRole = "realm-controller";
      transportBindingIds = [ "transport-public" "transport-broker" ];
    }
    {
      providerId = nativeVsockProviderId;
      inherit realmId;
      implementationId = "native-vsock";
      controllerRole = "realm-controller";
      transportBindingIds = [ "transport-guest" ];
    }
  ];

  substrateEntries = substrate.mkEntries [
    {
      providerId = nixosProviderId;
      realmId = localRootRealmId;
      implementationId = "nixos";
    }
    {
      providerId = linuxProviderId;
      realmId = localRootRealmId;
      implementationId = "linux";
    }
  ];

  displayEntries = display.mkEntries [{
    providerId = waylandProviderId;
    inherit realmId workloadId ownerRoleId;
    implementationId = "wayland";
    controllerRole = "realm-controller";
    endpointIds = {
      wayland = "wayland-${ownerRoleId}";
      crossDomain = "cross-domain-${ownerRoleId}";
      waypipe = "waypipe-${ownerRoleId}";
      proxy = "proxy-${ownerRoleId}";
    };
  }];

  integratedIndex = (lib.evalModules {
    modules = [
      (flakeRoot + "/nixos-modules/index.nix")
      ({ lib, ... }: {
        options.d2b.realms = lib.mkOption {
          type = lib.types.attrs;
          default = { };
        };
        config.d2b.realms.local-root = {
          enable = true;
          id = "local-root";
          path = "local-root";
          placement = "host-local";
          providers = {
            runtime = {
              type = "runtime";
              implementationId = "cloud-hypervisor";
            };
            transport = {
              type = "transport";
              implementationId = "unix-stream";
            };
            substrate = {
              type = "substrate";
              implementationId = "nixos";
            };
            display = {
              type = "display";
              implementationId = "wayland";
            };
          };
          workloads.editor = {
            enable = true;
            id = "editor";
            providerRefs.display = "display";
            runtime = {
              provider = "runtime";
              implementation = "cloud-hypervisor";
            };
            graphics.enable = true;
          };
        };
      })
    ];
  }).config.d2b._index;
  integratedTransport = import
    "${flakeRoot}/nixos-modules/provider-registry-v2-extensions/transport.nix"
    { inherit lib identity; cfg._index = integratedIndex; generation = 7; };
  integratedSubstrate = import
    "${flakeRoot}/nixos-modules/provider-registry-v2-extensions/substrate.nix"
    { inherit lib identity; cfg._index = integratedIndex; generation = 7; };
  integratedDisplay = import
    "${flakeRoot}/nixos-modules/provider-registry-v2-extensions/display.nix"
    { inherit lib identity; cfg._index = integratedIndex; generation = 7; };

  normalizedCfg = {
    _index = {
      realms.enabledList = [
        {
          realmId = localRootRealmId;
          placement = "host-local";
          parentRealmId = null;
          parentPath = null;
        }
        {
          inherit realmId;
          placement = "host-local";
          parentRealmId = localRootRealmId;
          parentPath = "local-root";
        }
      ];
      workloads.enabledList = [{
        inherit realmId workloadId;
      }];
      roles.enabledList = [{
        inherit realmId workloadId;
        roleId = ownerRoleId;
        roleKind = "wayland-proxy";
      }];
      providers.enabledList = [
        {
          providerId = unixStreamProviderId;
          inherit realmId;
          providerType = "transport";
          implementationId = "unix-stream";
          placement = "host-local";
        }
        {
          providerId = nativeVsockProviderId;
          inherit realmId;
          providerType = "transport";
          implementationId = "native-vsock";
          placement = "host-local";
        }
        {
          providerId = nixosProviderId;
          realmId = localRootRealmId;
          providerType = "substrate";
          implementationId = "nixos";
          placement = "host-local";
        }
        {
          providerId = linuxProviderId;
          realmId = localRootRealmId;
          providerType = "substrate";
          implementationId = "linux";
          placement = "host-local";
        }
        {
          providerId = waylandProviderId;
          inherit realmId;
          providerType = "display";
          implementationId = "wayland";
          placement = "host-local";
        }
      ];
      providerRegistryV2Mappings = {
        transport = [
          {
            providerId = unixStreamProviderId;
            inherit realmId;
            implementationId = "unix-stream";
            controllerRole = "realm-controller";
            transportBindingIds = [ "transport-${unixStreamProviderId}" ];
          }
          {
            providerId = nativeVsockProviderId;
            inherit realmId;
            implementationId = "native-vsock";
            controllerRole = "realm-controller";
            transportBindingIds = [ "transport-${nativeVsockProviderId}" ];
          }
        ];
        substrate = [
          {
            providerId = nixosProviderId;
            realmId = localRootRealmId;
            implementationId = "nixos";
            controllerRole = "local-root-controller";
          }
          {
            providerId = linuxProviderId;
            realmId = localRootRealmId;
            implementationId = "linux";
            controllerRole = "local-root-controller";
          }
        ];
        display = [{
          providerId = waylandProviderId;
          inherit realmId workloadId ownerRoleId;
          implementationId = "wayland";
          controllerRole = "realm-controller";
          endpointIds = {
            wayland = "wayland-${ownerRoleId}";
            crossDomain = "cross-domain-${ownerRoleId}";
            waypipe = "waypipe-${ownerRoleId}";
            proxy = "proxy-${ownerRoleId}";
          };
        }];
      };
    };
  };
  configuredTransport = import
    "${flakeRoot}/nixos-modules/provider-registry-v2-extensions/transport.nix"
    { inherit lib identity; cfg = normalizedCfg; generation = 7; };
  configuredSubstrate = import
    "${flakeRoot}/nixos-modules/provider-registry-v2-extensions/substrate.nix"
    { inherit lib identity; cfg = normalizedCfg; generation = 7; };
  configuredDisplay = import
    "${flakeRoot}/nixos-modules/provider-registry-v2-extensions/display.nix"
    { inherit lib identity; cfg = normalizedCfg; generation = 7; };

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
        transportBindingIds = [ "transport-broker" "transport-public" ];
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
          wayland = "wayland-${ownerRoleId}";
          crossDomain = "cross-domain-${ownerRoleId}";
          waypipe = "waypipe-${ownerRoleId}";
          proxy = "proxy-${ownerRoleId}";
        };
      };
    };
  };

  "platform-provider-mappings/configured-provider-fragments" = {
    expr = {
      transportCount = builtins.length configuredTransport.providers;
      transportProviderIds =
        map (entry: entry.descriptor.providerId) configuredTransport.providers;
      substrateImplementations = map
        (entry: entry.descriptor.implementationId)
        configuredSubstrate.providers;
      displayCount = builtins.length configuredDisplay.providers;
      displayBinding = (builtins.head configuredDisplay.providers).binding;
    };
    expected = {
      transportCount = 2;
      transportProviderIds = lib.sort lib.lessThan [
        unixStreamProviderId
        nativeVsockProviderId
      ];
      substrateImplementations = [ "linux" "nixos" ];
      displayCount = 1;
      displayBinding = {
        axis = "local-display";
        inherit workloadId ownerRoleId;
        endpointIds = {
          wayland = "wayland-${ownerRoleId}";
          crossDomain = "cross-domain-${ownerRoleId}";
          waypipe = "waypipe-${ownerRoleId}";
          proxy = "proxy-${ownerRoleId}";
        };
      };
    };
  };

  "platform-provider-mappings/integrated-index-populates-resource-backed-providers" = {
    expr =
      let
        mappings = integratedIndex.providerRegistryV2Mappings;
        transportProviders = integratedTransport.providers;
        substrateProviders = integratedSubstrate.providers;
        displayProviders = integratedDisplay.providers;
        resourceIds = integratedIndex.resources.byId;
        mappedResourceIds =
          lib.concatMap (mapping: mapping.transportBindingIds) mappings.transport
          ++ lib.concatMap
            (mapping: lib.attrValues mapping.endpointIds)
            mappings.display;
        placementMatches = mapping: entry:
          entry.descriptor.providerId == mapping.providerId
          && entry.descriptor.implementationId == mapping.implementationId
          && entry.descriptor.placement == {
            kind = "trusted-first-party-in-process";
            inherit (mapping) realmId controllerRole;
          };
      in
      {
        counts = {
          transport = builtins.length transportProviders;
          substrate = builtins.length substrateProviders;
          display = builtins.length displayProviders;
        };
        transport = placementMatches
          (builtins.head mappings.transport)
          (builtins.head transportProviders)
          && (builtins.head transportProviders).binding == {
            axis = "local-transport";
            inherit ((builtins.head mappings.transport)) transportBindingIds;
          };
        substrate =
          let
            mapping = builtins.head mappings.substrate;
            entry = builtins.head substrateProviders;
          in
          entry.descriptor.providerId == mapping.providerId
          && entry.descriptor.implementationId == mapping.implementationId
          && entry.descriptor.placement == {
            kind = "trusted-first-party-in-process";
            inherit (mapping) realmId controllerRole;
          }
          && entry.binding == { axis = "local-substrate"; };
        display = placementMatches
          (builtins.head mappings.display)
          (builtins.head displayProviders)
          && (builtins.head displayProviders).binding == {
            axis = "local-display";
            inherit ((builtins.head mappings.display))
              workloadId ownerRoleId endpointIds;
          };
        resourcesBackEveryOpaqueId =
          lib.all (resourceId: builtins.hasAttr resourceId resourceIds)
            mappedResourceIds;
      };
    expected = {
      counts = {
        transport = 1;
        substrate = 1;
        display = 1;
      };
      transport = true;
      substrate = true;
      display = true;
      resourcesBackEveryOpaqueId = true;
    };
  };

  "platform-provider-mappings/reject-missing-authoritative-mapping-seam" = {
    expr =
      let
        missing = path: import path {
          inherit lib identity;
          cfg._index = { };
          generation = 7;
        };
      in
      builtins.deepSeq [
        (missing
          "${flakeRoot}/nixos-modules/provider-registry-v2-extensions/transport.nix").providers
        (missing
          "${flakeRoot}/nixos-modules/provider-registry-v2-extensions/substrate.nix").providers
        (missing
          "${flakeRoot}/nixos-modules/provider-registry-v2-extensions/display.nix").providers
      ] true;
    expectedError = { };
  };

  "platform-provider-mappings/reject-unregistered-transport" = {
    expr = transport.mkEntries [{
      providerId = providerId "transport" "loopback";
      inherit realmId;
      implementationId = "loopback";
      controllerRole = "realm-controller";
      transportBindingIds = [ "transport-test" ];
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
        wayland = "wayland-${ownerRoleId}";
        crossDomain = "cross-domain-${ownerRoleId}";
        waypipe = "waypipe-${ownerRoleId}";
        proxy = "proxy-${ownerRoleId}";
      };
    }];
    expectedError = { };
  };

  "platform-provider-mappings/reject-display-placement" = {
    expr = display.mkEntries [{
      providerId = waylandProviderId;
      inherit realmId workloadId ownerRoleId;
      implementationId = "wayland";
      controllerRole = "provider-agent";
      endpointIds = {
        wayland = "wayland-${ownerRoleId}";
        crossDomain = "cross-domain-${ownerRoleId}";
        waypipe = "waypipe-${ownerRoleId}";
        proxy = "proxy-${ownerRoleId}";
      };
    }];
    expectedError = { };
  };
}
