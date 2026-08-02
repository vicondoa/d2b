# Semantic Service/Binding and cross-Zone sharing compiler coverage.
{ mkEval, lib, pkgs, ... }:

let
  resourceBundle = import ../../../../nixos-modules/resources-bundle.nix { inherit lib; };
  providerIds = [
    "audio-pipewire"
    "device-usbip"
    "device-security-key"
  ];
  artifacts = lib.listToAttrs (map
    (id: lib.nameValuePair id {
      package = pkgs.writeText "provider-${id}" id;
      type = "provider";
    })
    providerIds);

  provider = artifactId: {
    type = "Provider";
    spec = {
      inherit artifactId;
      config = { };
    };
  };

  providerValidationFailures = resources:
    lib.filter (record: !record.assertion)
      (resourceBundle.validateBundle "work" resources).assertions;

  safeProviderConfig = {
    provider = {
      type = "Provider";
      spec = {
        artifactId = "provider-safe";
        config = {
          selfMetrics.enable = true;
          timeoutMs = 1500;
          mode = "bounded";
          endpointRef = "Endpoint/provider";
        };
      };
    };
  };

  inlineProviderSecrets = {
    provider = {
      type = "Provider";
      spec = {
        artifactId = "provider-unsafe";
        config = {
          token = "inline-token";
          password = "inline-password";
          privateKey = "inline-private-key";
          path = "/nix/store/not-a-secret";
          argv = [ "provider" ];
        };
      };
    };
  };

  guest = {
    type = "Guest";
    spec = {
      providerRef = "Provider/audio-pipewire";
    };
  };

  ownerService = {
    type = "audio.d2bus.org.AudioService";
    spec = {
      providerRef = "Provider/audio-pipewire";
      serviceRole = "authority";
      implementationEndpointRefs = [ ];
      operations = [ "capture" ];
    };
  };

  ownerBinding = {
    type = "audio.d2bus.org.AudioBinding";
    spec = {
      providerRef = "Provider/audio-pipewire";
      serviceRef = "audio.d2bus.org.AudioService/host-audio";
      grants = { };
      targetRef = "Guest/workstation";
    };
  };

  ownerExport = {
    type = "ResourceExport";
    spec = {
      providerRef = "Provider/audio-pipewire";
      resourceRef = "audio.d2bus.org.AudioService/host-audio";
      serviceType = "audio.d2bus.org.AudioService";
      projectionSchemaFingerprint = "sha256:e0aafd9924df4b1bf21792a0c4fc7b8d1637dde960dfae33e9d7de2452028690";
      factoryFingerprint = "sha256:80ef3d08378a61ac924944564efa136b0cfba314d1e48567680d16cc75ac4b38";
      operations = [ "capture" ];
      arbitration = "exclusive";
      quota = { };
      consumerZonePolicy = {
        zones = [ "Zone/work" ];
        capabilityCeiling = [ "capture" ];
      };
      visibility = "named-zones";
      updatePolicy = { mode = "manual-disruptive"; };
      revocationPolicy = { };
    };
  };

  importResource = {
    type = "ResourceImport";
    spec = {
      providerRef = "Provider/audio-pipewire";
      zoneLinkRef = "ZoneLink/work-uplink";
      exportKey = "host/host-audio";
      expectedServiceType = "audio.d2bus.org.AudioService";
      expectedProjectionSchemaFingerprint = "sha256:e0aafd9924df4b1bf21792a0c4fc7b8d1637dde960dfae33e9d7de2452028690";
      expectedFactoryFingerprint = "sha256:80ef3d08378a61ac924944564efa136b0cfba314d1e48567680d16cc75ac4b38";
      projectionName = "host-audio";
      requestedCapabilities = [ "capture" ];
      requestedQuota = { };
      updatePolicy = { mode = "manual-disruptive"; };
      disconnectPolicy = { mode = "degrade"; };
    };
  };

  base = { ... }: {
    d2b.artifacts = artifacts;
    d2b.zones.local-root.resources = {
      audio-pipewire = provider "audio-pipewire";
      device-usbip = provider "device-usbip";
      device-security-key = provider "device-security-key";
      workstation = guest;
      host-audio = ownerService;
      host-audio-binding = ownerBinding;
      host-audio-export = ownerExport;
    };
    d2b.zones.work = {
      parentZone = "local-root";
      resources = {
        audio-pipewire = provider "audio-pipewire";
        workstation = guest;
        work-uplink = {
          type = "ZoneLink";
          spec = {
            childZoneName = "work";
            disabled = false;
            limits = { };
            transportCredentials = [ ];
            transportProviderRef = "Provider/audio-pipewire";
            transportSettings = { };
          };
        };
        host-audio-import = importResource;
      };
    };
  };

  providerSchema = {
    configSchema = {
      type = "object";
      additionalProperties = true;
      properties = {
        timeoutMs = {
          type = "integer";
          minimum = 1;
          maximum = 5000;
        };
      };
    };
  };
  providerSchemas = lib.listToAttrs (map
    (id: lib.nameValuePair id providerSchema)
    providerIds);
  providerValidationRecords = providerConfig:
    let
      evaluated = (mkEval [
        base
        ({ ... }: {
          d2b._providerSettingsValidation.enable = true;
          d2b._providerSettingsValidation.schemas = providerSchemas;
          d2b.zones.local-root.resources.audio-pipewire.spec.config =
            providerConfig;
        })
      ]).config;
    in
    evaluated.d2b._resourceCompiler.providerSettingsValidation.assertions;

  cfg = (mkEval [ base ]).config;
  failures = configuration:
    map (assertion: assertion.message)
      (lib.filter (assertion: !assertion.assertion) configuration.assertions);

  invalid = override:
    failures ((mkEval [ base override ]).config);
in
{
  "resource-sharing/authority-service-and-binding-are-local" = {
    expr = {
      service = cfg.d2b._resourceCompiler.sharing.serviceTypes;
      binding = cfg.d2b._resourceCompiler.sharing.bindingTypes;
      projection = cfg.d2b._resourceCompiler.sharing.projectionsByZone.work.host-audio.spec;
    };
    expected = {
      service = [
        "audio.d2bus.org.AudioService"
        "security-key.d2bus.org.SecurityKeyService"
        "telemetry.d2bus.org.TelemetryService"
        "usb.d2bus.org.UsbService"
      ];
      binding = [
        "audio.d2bus.org.AudioBinding"
        "security-key.d2bus.org.SecurityKeyBinding"
        "telemetry.d2bus.org.TelemetryBinding"
        "usb.d2bus.org.UsbBinding"
      ];
      projection = {
        providerRef = "Provider/audio-pipewire";
        serviceRole = "projection";
        implementationEndpointRefs = [ ];
      };
    };
  };

  "resource-sharing/no-authored-projection-service" = {
    expr = lib.any (message: lib.hasInfix "projection Service" message)
      (invalid {
        d2b.zones.local-root.resources.bad-projection = {
          type = "audio.d2bus.org.AudioService";
          spec = ownerService.spec // { serviceRole = "projection"; };
        };
      });
    expected = true;
  };

  "resource-sharing/export-is-service-only" = {
    expr = lib.any (message: lib.hasInfix "local owner Service" message)
      (invalid {
        d2b.zones.local-root.resources.host-audio-export.spec.resourceRef =
          "Device/physical-mic";
      });
    expected = true;
  };

  "resource-sharing/binding-target-must-be-same-zone" = {
    expr = lib.any (message: lib.hasInfix "same-Zone target" message)
      (invalid {
        d2b.zones.local-root.resources.host-audio-binding.spec.targetRef =
          "Guest/workstation";
      });
    expected = true;
  };

  "resource-sharing/provider-config-secret-lint-uses-canonical-record" = {
    expr = {
      safe = providerValidationFailures safeProviderConfig == [ ];
      unsafe = lib.any
        (record:
          lib.hasInfix
            "contains a secret, path, argv, PID, or UID-shaped value"
            record.message)
        (providerValidationFailures inlineProviderSecrets);
    };
    expected = {
      safe = true;
      unsafe = true;
    };
  };

  "resource-sharing/provider-settings-validation-sees-full-config" = {
    expr = {
      safe = lib.all (record: record.assertion)
        (providerValidationRecords {
          timeoutMs = 1500;
          mode = "bounded";
        });
      unsafe = lib.any
        (record:
          !record.assertion
          && lib.hasInfix "spec.config contains secret-shaped key/value material"
            record.message)
        (providerValidationRecords { token = "inline-token"; });
    };
    expected = {
      safe = true;
      unsafe = true;
    };
  };

  "resource-sharing/security-key-backing-fails-closed" = {
    expr = lib.any
      (message: lib.hasInfix "refusing to invent" message)
      (invalid {
        d2b.zones.local-root.resources.security-key = {
          type = "security-key.d2bus.org.SecurityKeyService";
          spec = {
            providerRef = "Provider/device-security-key";
            mode = "authority";
          };
        };
        d2b.zones.local-root.resources.security-key-export = {
          type = "ResourceExport";
          spec = ownerExport.spec // {
            providerRef = "Provider/device-security-key";
            resourceRef = "security-key.d2bus.org.SecurityKeyService/security-key";
            serviceType = "security-key.d2bus.org.SecurityKeyService";
          };
        };
      });
    expected = true;
  };

  "resource-sharing/usb-export-needs-all-policy-opt-ins" = {
    expr = lib.any
      (message: lib.hasInfix "policy opt-in" message)
      (invalid {
        d2b.zones.local-root.resources.usb-service = {
          type = "usb.d2bus.org.UsbService";
          spec = {
            providerRef = "Provider/device-usbip";
            mode = "authority";
            accessPolicy = { };
          };
        };
        d2b.zones.local-root.resources.usb-export = {
          type = "ResourceExport";
          spec = ownerExport.spec // {
            providerRef = "Provider/device-usbip";
            resourceRef = "usb.d2bus.org.UsbService/usb-service";
            serviceType = "usb.d2bus.org.UsbService";
            projectionSchemaFingerprint = "sha256:dc40fd25748ca024cdd03f74566494f949de8531046ee997cf0e6787e58b34c6";
            factoryFingerprint = "sha256:72b5cafbd2409d187b523b1d6076094f8d6246d0a5714240d1b7bac775ed7b45";
          };
        };
      });
    expected = true;
  };
}
