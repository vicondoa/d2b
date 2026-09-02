# Nix-unit coverage for child-local ZoneLink and gateway Guest contracts.
{ mkModuleEval, lib, pkgs, ... }:

let
  gatewaySystem = pkgs.writeText "zone-link-gateway-system" "gateway-system";
  providerPackage = name: pkgs.writeText "zone-link-${name}" name;

  assertionsModule = { lib, ... }: {
    options.assertions = lib.mkOption {
      type = lib.types.listOf lib.types.attrs;
      default = [ ];
    };
  };

  eval = extra:
    (mkModuleEval [ assertionsModule base extra ]).config;

  base = { ... }: {
    d2b.artifacts = {
      gateway-system = {
        package = gatewaySystem;
        type = "nixos-system";
      };
      runtime-cloud-hypervisor = {
        package = providerPackage "runtime-cloud-hypervisor";
        type = "provider";
      };
      runtime-azure-container-apps = {
        package = providerPackage "runtime-azure-container-apps";
        type = "provider";
      };
      transport-unix = {
        package = providerPackage "transport-unix";
        type = "provider";
      };
      transport-azure-relay = {
        package = providerPackage "transport-azure-relay";
        type = "provider";
      };
      credential-managed-identity = {
        package = providerPackage "credential-managed-identity";
        type = "provider";
      };
    };

    d2b.guestSystems.local-zone.gateway.config.system.build.toplevel =
      gatewaySystem;
    d2b.guestSystems.gateway-zone.gateway.config.system.build.toplevel =
      gatewaySystem;

    d2b.zones.local-root = {
      resources = { };
    };

    d2b.zones.local-zone = {
      parentZone = "local-root";
      resources = {
        host = {
          type = "Host";
          spec.providerRef = "Provider/system-core";
        };
        gateway = {
          type = "Guest";
          spec = {
            providerRef = "Provider/runtime-cloud-hypervisor";
            systemArtifactId = "gateway-system";
          };
        };
        egress = {
          type = "Network";
          spec = {
            lanCidr = "10.70.0.0/24";
          };
        };
        runtime-cloud-hypervisor = {
          type = "Provider";
          spec = {
            artifactId = "runtime-cloud-hypervisor";
            config = {
              controllerExecutionRef = "Host/host";
            };
          };
        };
        transport-unix = {
          type = "Provider";
          spec = {
            artifactId = "transport-unix";
            config = {
              executionRef = "Host/host";
            };
          };
        };
        local-uplink = {
          type = "ZoneLink";
          spec = {
            childZoneName = "local-zone";
            disabled = false;
            limits = {
              maxActiveStreams = 32;
              maxPendingIntents = 256;
              reconnectMaxAttempts = 10;
              reconnectWindowSecs = 300;
            };
            transportCredentials = [ ];
            transportProviderRef = "Provider/transport-unix";
            transportSettings = {
              socketKind = "seqpacket";
            };
          };
        };
      };
    };

    d2b.zones.gateway-zone = {
      parentZone = "local-root";
      resources = {
        host = {
          type = "Host";
          spec.providerRef = "Provider/system-core";
        };
        gateway = {
          type = "Guest";
          spec = {
            providerRef = "Provider/runtime-cloud-hypervisor";
            systemArtifactId = "gateway-system";
            networkAttachments = [
              {
                default = true;
                networkRef = "Network/egress";
              }
            ];
          };
        };
        egress = {
          type = "Network";
          spec = {
            externalAttachment = {
              ipv4 = {
                address = "192.0.2.2/30";
                dns = [ "192.0.2.53" ];
                gateway = "192.0.2.1";
                method = "static";
              };
            };
            lanCidr = "10.71.0.0/24";
          };
        };
        runtime-cloud-hypervisor = {
          type = "Provider";
          spec = {
            artifactId = "runtime-cloud-hypervisor";
            config = {
              controllerExecutionRef = "Host/host";
            };
          };
        };
        runtime-azure-container-apps = {
          type = "Provider";
          spec = {
            artifactId = "runtime-azure-container-apps";
            config = {
              controlCredentialRef = "Credential/aca-control";
              gatewayExecutionRef = "Guest/gateway";
              networkRef = "Network/egress";
              pullCredentialRef = null;
            };
          };
        };
        transport-azure-relay = {
          type = "Provider";
          spec = {
            artifactId = "transport-azure-relay";
            config = {
              executionRef = "Guest/gateway";
              networkRef = "Network/egress";
            };
          };
        };
        credential-managed-identity = {
          type = "Provider";
          spec = {
            artifactId = "credential-managed-identity";
            config = {
              credentialDomains = [ "system" ];
              supportedOperations = [ "acquire-token" ];
            };
          };
        };
        aca-control = {
          type = "Credential";
          spec = {
            allowedOperations = [ "acquire-token" ];
            audience = "https://management.azure.com/";
            consumerRef = "Provider/runtime-azure-container-apps";
            providerRef = "Provider/credential-managed-identity";
            scope.executionRef = "Guest/gateway";
          };
        };
        relay-listen = {
          type = "Credential";
          spec = {
            allowedOperations = [ "acquire-token" ];
            audience = "azure-relay-listen";
            consumerRef = "Provider/transport-azure-relay";
            providerRef = "Provider/credential-managed-identity";
            scope.executionRef = "Guest/gateway";
          };
        };
        relay-send = {
          type = "Credential";
          spec = {
            allowedOperations = [ "acquire-token" ];
            audience = "azure-relay-send";
            consumerRef = "Provider/transport-azure-relay";
            providerRef = "Provider/credential-managed-identity";
            scope.executionRef = "Guest/gateway";
          };
        };
        aca-sandbox = {
          type = "Guest";
          spec = {
            allowedDomains = [ "system" ];
            defaultDomain = "system";
            provider = {
              settings = {
                configuredImageId = "aca-image";
              };
            };
            providerRef = "Provider/runtime-azure-container-apps";
          };
        };
        gateway-uplink = {
          type = "ZoneLink";
          spec = {
            childZoneName = "gateway-zone";
            disabled = false;
            limits = {
              maxActiveStreams = 32;
              maxPendingIntents = 256;
              reconnectMaxAttempts = 10;
              reconnectWindowSecs = 300;
            };
            transportCredentials = [
              "Credential/relay-listen"
              "Credential/relay-send"
            ];
            transportProviderRef = "Provider/transport-azure-relay";
            transportSettings = {
              relayEntityId = "gateway";
              relayNamespaceId = "relns-d2b-prod";
            };
          };
        };
      };
    };
  };

  bundleResources = cfg: zone:
    cfg.d2b._bundle.zoneResourceBundlesV3.${zone}.data.resources;

  findResource = type: name: resources:
    lib.findFirst
      (resource:
        resource.type == type && resource.metadata.name == name)
      null
      resources;

  failureMessages = extra:
    let cfg = eval extra;
    in map (assertion: assertion.message)
      (lib.filter (assertion: !assertion.assertion) cfg.assertions);

  hasFailure = needle: extra:
    lib.any (message: lib.hasInfix needle message)
      (failureMessages extra);

  disabledProjection = eval {
    d2b._resourceCompiler.providerProjectionRuntimeAzureContainerApps =
      lib.mkForce {
        enabled = false;
        guestPatchesByZone.gateway-zone.gateway = {
          provider = {
            disabled-provider-secret-canary = true;
          };
        };
        privateArtifact = {
          credential = "disabled-provider-secret-canary";
        };
        processesByZone.gateway-zone.disabled-process = {
          metadata = {
            name = "disabled-process";
            ownerRef = "Provider/runtime-azure-container-apps";
            zone = "gateway-zone";
          };
          spec = {
            executionRef = "Guest/gateway";
            processClass = "service";
            providerRef = "Provider/system-systemd";
            template = "disabled-process";
          };
          type = "Process";
        };
        resourcesByZone.gateway-zone.disabled-resource = {
          metadata = {
            name = "disabled-resource";
            zone = "gateway-zone";
          };
          spec = { };
          type = "User";
        };
      };
  };
in
{
  "zone-link/renders-local-and-gateway-projections" =
    let
      cfg = eval { };
      local = bundleResources cfg "local-zone";
      gateway = bundleResources cfg "gateway-zone";
      localLink = findResource "ZoneLink" "local-uplink" local;
      gatewayLink = findResource "ZoneLink" "gateway-uplink" gateway;
      gatewayGuest = findResource "Guest" "gateway" gateway;
      acaProvider = findResource "Provider" "runtime-azure-container-apps" gateway;
      relayProvider = findResource "Provider" "transport-azure-relay" gateway;
      network = findResource "Network" "egress" gateway;
      root = bundleResources cfg "local-root";
      allBundleData = lib.mapAttrs (_: value: value.data)
        cfg.d2b._bundle.zoneResourceBundlesV3;
    in {
      expr = {
        local = {
          childZoneName = localLink.spec.childZoneName;
          provider = localLink.spec.transportProviderRef;
          settings = localLink.spec.transportSettings.socketKind;
        };
        gateway = {
          childZoneName = gatewayLink.spec.childZoneName;
          credentials = gatewayLink.spec.transportCredentials;
          provider = gatewayLink.spec.transportProviderRef;
          settings = gatewayLink.spec.transportSettings;
        };
        gatewayExecution = {
          guestProvider = gatewayGuest.spec.providerRef;
          acaGateway = acaProvider.spec.config.gatewayExecutionRef;
          relayExecution = relayProvider.spec.config.executionRef;
          networkGateway =
            network.spec.externalAttachment.ipv4.gateway;
        };
        rootHasNoLink = findResource "ZoneLink" "gateway-uplink" root == null;
        parentZoneNotEmitted = !(builtins.hasAttr "parentZone"
          gatewayLink.spec);
        noCredentialBytes = !(lib.hasInfix "SharedAccessKey"
          (builtins.toJSON allBundleData));
      };
      expected = {
        local = {
          childZoneName = "local-zone";
          provider = "Provider/transport-unix";
          settings = "seqpacket";
        };
        gateway = {
          childZoneName = "gateway-zone";
          credentials = [
            "Credential/relay-listen"
            "Credential/relay-send"
          ];
          provider = "Provider/transport-azure-relay";
          settings = {
            relayEntityId = "gateway";
            relayNamespaceId = "relns-d2b-prod";
          };
        };
        gatewayExecution = {
          guestProvider = "Provider/runtime-cloud-hypervisor";
          acaGateway = "Guest/gateway";
          relayExecution = "Guest/gateway";
          networkGateway = "192.0.2.1";
        };
        rootHasNoLink = true;
        parentZoneNotEmitted = true;
        noCredentialBytes = true;
      };
    };

  "zone-link/same-resource-names-remain-zone-local" =
    let
      cfg = eval { };
      local = bundleResources cfg "local-zone";
      gateway = bundleResources cfg "gateway-zone";
      localGateway = findResource "Guest" "gateway" local;
      gatewayGateway = findResource "Guest" "gateway" gateway;
      localHost = findResource "Host" "host" local;
      gatewayHost = findResource "Host" "host" gateway;
    in {
      expr = {
        guestZones = [
          localGateway.metadata.zone
          gatewayGateway.metadata.zone
        ];
        hostZones = [
          localHost.metadata.zone
          gatewayHost.metadata.zone
        ];
        distinctBundleResources =
          localGateway != gatewayGateway && localHost != gatewayHost;
      };
      expected = {
        guestZones = [ "local-zone" "gateway-zone" ];
        hostZones = [ "local-zone" "gateway-zone" ];
        distinctBundleResources = true;
      };
    };

  "zone-link/missing-transport-provider-refuses" = {
    expr = hasFailure "same-Zone transport Provider ref" {
      d2b.zones.gateway-zone.resources.gateway-uplink.spec.transportProviderRef =
        lib.mkForce "Provider/missing";
    };
    expected = true;
  };

  "zone-link/cross-zone-transport-provider-refuses" = {
    expr = hasFailure "same-Zone transport Provider ref" {
      d2b.zones.gateway-zone.resources.gateway-uplink.spec.transportProviderRef =
        lib.mkForce "Provider/transport-unix";
    };
    expected = true;
  };

  "zone-link/cross-zone-credential-refuses" = {
    expr = hasFailure "same-Zone Credential refs" {
      d2b.zones.local-zone.resources.local-uplink.spec.transportCredentials =
        lib.mkForce [ "Credential/relay-listen" ];
    };
    expected = true;
  };

  "zone-link/child-name-must-match-zone" = {
    expr = hasFailure "childZoneName must equal the enclosing Zone name" {
      d2b.zones.gateway-zone.resources.gateway-uplink.spec.childZoneName =
        lib.mkForce "local-zone";
    };
    expected = true;
  };

  "zone-link/transport-settings-secret-value-refuses" = {
    expr = hasFailure "must not contain credential or locator fields" {
      d2b.zones.gateway-zone.resources.gateway-uplink.spec = lib.mkForce {
        childZoneName = "gateway-zone";
        disabled = false;
        limits = {
          maxActiveStreams = 32;
          maxPendingIntents = 256;
          reconnectMaxAttempts = 10;
          reconnectWindowSecs = 300;
        };
        transportCredentials = [
          "Credential/relay-listen"
          "Credential/relay-send"
        ];
        transportProviderRef = "Provider/transport-azure-relay";
        transportSettings = {
          relayEntityId = "SharedAccessSignature sr=canary";
          relayNamespaceId = "relns-d2b-prod";
        };
      };
    };
    expected = true;
  };

  "zone-link/provider-config-secret-material-refuses" = {
    expr = hasFailure "Provider config must not contain credential material" {
      d2b.zones.gateway-zone.resources.transport-azure-relay.spec = lib.mkForce {
        artifactId = "transport-azure-relay";
        config = {
          executionRef = "Guest/gateway";
          networkRef = "SharedAccessSignature sr=canary";
          maxConcurrentSessions = 32;
          connectTimeoutSeconds = 30;
        };
      };
    };
    expected = true;
  };

  "zone-link/gateway-settings-secret-material-refuses" = {
    expr = hasFailure "spec.provider.settings must not contain credential material" {
      d2b.zones.gateway-zone.resources.aca-sandbox.spec = lib.mkForce {
        allowedDomains = [ "system" ];
        defaultDomain = "system";
        provider = {
          settings = {
            configuredImageId = "SharedAccessSignature sr=canary";
          };
        };
        providerRef = "Provider/runtime-azure-container-apps";
      };
    };
    expected = true;
  };

  "zone-link/gateway-provider-cannot-use-host" = {
    expr = hasFailure "must resolve to the gateway Guest" {
      d2b.zones.gateway-zone.resources.runtime-azure-container-apps.spec.config.gatewayExecutionRef =
        lib.mkForce "Host/host";
    };
    expected = true;
  };

  "zone-link/disabled-provider-projection-emits-nothing" =
    let
      cfg = disabledProjection;
      resources = bundleResources cfg "gateway-zone";
      guest = findResource "Guest" "gateway" resources;
      artifactRows = cfg.d2b._bundle.extraArtifacts;
    in {
      expr = {
        processAbsent =
          findResource "Process" "disabled-process" resources == null;
        resourceAbsent =
          findResource "User" "disabled-resource" resources == null;
        guestUnpatched =
          !(builtins.hasAttr "provider" guest.spec)
          || !(builtins.hasAttr "disabled-provider-secret-canary"
            guest.spec.provider);
        artifactAbsent =
          !(builtins.hasAttr "runtime-azure-container-apps" artifactRows);
      };
      expected = {
        processAbsent = true;
        resourceAbsent = true;
        guestUnpatched = true;
        artifactAbsent = true;
      };
    };

}
