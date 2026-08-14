# Focused positive and negative coverage for the eval-time Provider runtime
# boundary assertions.
{ lib, ... }@ctx:

let
  providerRuntimeContracts =
    import ../../../../nixos-modules/provider-runtime-contracts.nix;

  mkEvalContracts = modules:
    lib.evalModules {
      modules = [
        providerRuntimeContracts
        {
          options.assertions = lib.mkOption {
            type = lib.types.listOf lib.types.anything;
            default = [ ];
          };
          options.d2b.zones = lib.mkOption {
            type = lib.types.attrsOf lib.types.anything;
            default = { };
          };
        }
      ] ++ modules;
    };

  contractBase = { ... }: {
    d2b.zones.local-root.resources = {
      host = {
        type = "Host";
        spec = { };
      };
      gateway = {
        type = "Guest";
        spec = {
          providerRef = "Provider/runtime-azure-container-apps";
          gateway = { };
        };
      };
      control-network = {
        type = "Network";
        spec = { };
      };
      system = {
        type = "Provider";
        spec = {
          artifactId = "system";
          config = { };
        };
      };
      runtime-azure-container-apps = {
        type = "Provider";
        spec = {
          config = {
            gatewayExecutionRef = "Guest/gateway";
            controlCredentialRef = "Credential/aca-control";
            pullCredentialRef = "Credential/aca-pull";
            networkRef = "Network/control-network";
          };
        };
      };
      runtime-azure-virtual-machine = {
        type = "Provider";
        spec = {
          config = {
            controllerExecutionRef = "Guest/gateway";
            armCredentialRef = "Credential/vm-arm";
            networkRef = "Network/control-network";
          };
        };
      };
      runtime-cloud-hypervisor = {
        type = "Provider";
        spec = {
          config = {
            controllerExecutionRef = "Host/host";
          };
        };
      };
      transport-azure-relay = {
        type = "Provider";
        spec = {
          config = {
            executionRef = "Guest/gateway";
            networkRef = "Network/control-network";
          };
        };
      };
      aca-control = {
        type = "Credential";
        spec = {
          scope.executionRef = "Guest/gateway";
        };
      };
      aca-pull = {
        type = "Credential";
        spec = {
          scope.executionRef = "Guest/gateway";
        };
      };
      vm-arm = {
        type = "Credential";
        spec = {
          scope.executionRef = "Guest/gateway";
        };
      };
      system-guest = {
        type = "Guest";
        spec = {
          providerRef = "Provider/runtime-cloud-hypervisor";
          systemArtifactId = "system";
          provider.settings.memoryShared = true;
        };
      };
      relay-link = {
        type = "ZoneLink";
        spec = {
          childZoneName = "child";
          transportProviderRef = "Provider/transport-azure-relay";
          transportSettings = {
            relayNamespaceId = "relay-prod";
            relayEntityId = "gateway";
          };
          transportCredentials = [ ];
          disabled = false;
          limits = {
            maxActiveStreams = 32;
            maxPendingIntents = 256;
            reconnectMaxAttempts = 10;
            reconnectWindowSecs = 300;
          };
        };
      };
    };
    d2b.zones.child = {
      parentZone = "local-root";
      resources = { };
    };
  };

  failureMessages = modules:
    map (assertion: assertion.message)
      (lib.filter (assertion: !assertion.assertion)
        (mkEvalContracts modules).config.assertions);

  hasFailure = needle: modules:
    lib.any (message: lib.hasInfix needle message)
      (failureMessages modules);

  positive = mkEvalContracts [ contractBase ];
in
{
  "provider-runtime-contracts/accepts-valid-runtime-provider-bindings" = {
    expr = lib.filter (assertion: !assertion.assertion)
      positive.config.assertions;
    expected = [ ];
  };

  "provider-runtime-contracts/rejects-unknown-same-zone-provider-reference" = {
    expr = hasFailure "existing same-Zone runtime Provider" [
      contractBase
      ({ ... }: {
        d2b.zones.local-root.resources.gateway.spec.providerRef =
          lib.mkForce "Provider/runtime-cloud-hypervisor";
        d2b.zones.local-root.resources.runtime-cloud-hypervisor.type =
          lib.mkForce "Network";
      })
    ];
    expected = true;
  };

  "provider-runtime-contracts/rejects-cross-zone-provider-reference" = {
    expr = hasFailure "existing same-Zone runtime Provider" [
      contractBase
      ({ ... }: {
        d2b.zones.child.resources.gateway = {
          type = "Guest";
          spec.providerRef = "Provider/runtime-azure-container-apps";
        };
      })
    ];
    expected = true;
  };

  "provider-runtime-contracts-enforces-vm-arm-credential-scope" = {
    expr = hasFailure "ARM credential scope must match controllerExecutionRef" [
      contractBase
      ({ ... }: {
        d2b.zones.local-root.resources.vm-arm.spec.scope.executionRef =
          lib.mkForce "Guest/other";
      })
    ];
    expected = true;
  };

  "provider-runtime-contracts-requires-exact-relay-settings" = {
    expr = hasFailure "must contain exactly relayNamespaceId and relayEntityId" [
      contractBase
      ({ ... }: {
        d2b.zones.local-root.resources.relay-link.spec.transportSettings.extra =
          "rejected";
      })
    ];
    expected = true;
  };

  "provider-runtime-contracts-validates-relay-identifiers" = {
    expr = hasFailure "relayEntityId has an invalid Azure Relay entity shape" [
      contractBase
      ({ ... }: {
        d2b.zones.local-root.resources.relay-link.spec.transportSettings.relayEntityId =
          lib.mkForce "Not_An_Entity";
      })
    ];
    expected = true;
  };

  "provider-runtime-contracts-requires-provider-resolution-for-processes" = {
    expr = hasFailure "spec.providerRef must resolve to an existing same-Zone runtime Provider" [
      contractBase
      ({ ... }: {
        d2b.zones.local-root.resources.runtime-cloud-hypervisor.type =
          lib.mkForce "Network";
        d2b.zones.local-root.resources.worker = {
          type = "Process";
          spec = {
            providerRef = "Provider/runtime-cloud-hypervisor";
            executionRef = "Guest/gateway";
          };
        };
      })
    ];
    expected = true;
  };
}
