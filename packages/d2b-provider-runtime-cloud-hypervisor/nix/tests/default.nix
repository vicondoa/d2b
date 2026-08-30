{ lib, ... }:

let
  base = {
    options.assertions = lib.mkOption {
      type = lib.types.listOf lib.types.anything;
      default = [ ];
    };
    options.d2b.zones = lib.mkOption {
      type = lib.types.attrs;
      default = { };
    };
    options.d2b._resourceCompiler = lib.mkOption {
      type = lib.types.attrs;
      default = {
        volumeShorthand = { };
      };
      internal = true;
      visible = false;
    };
    options.d2b.artifacts = lib.mkOption {
      type = lib.types.attrs;
      default = { };
    };
    options.d2b._providerCatalog = lib.mkOption {
      type = lib.types.attrs;
      default = { };
      internal = true;
      visible = false;
    };
  };
  bundleBase = {
    options.d2b._artifactCatalogV3 = lib.mkOption {
      type = lib.types.attrs;
      default = { };
      internal = true;
      visible = false;
    };
    options.d2b._bundle.extraArtifacts = lib.mkOption {
      type = lib.types.attrs;
      default = { };
      internal = true;
      visible = false;
    };
    options.d2b._bundle.zoneResourceBundles = lib.mkOption {
      type = lib.types.attrs;
      default = { };
      internal = true;
      visible = false;
    };
  };
  providerPackage = builtins.toFile "d2b-runtime-cloud-hypervisor-provider" "provider";
  systemPackage = builtins.toFile "d2b-runtime-cloud-hypervisor-system" "system";
  guestResources = {
    host-system = {
      type = "Host";
      spec = { };
    };
    runtime-cloud-hypervisor = {
      type = "Provider";
      spec = {
        artifactId = "runtime-cloud-hypervisor";
        config.controllerExecutionRef = "Host/host-system";
      };
    };
    guest = {
      type = "Guest";
      spec = {
        providerRef = "Provider/runtime-cloud-hypervisor";
        systemArtifactId = "guest-system";
      };
    };
  };
  enabled = lib.evalModules {
    modules = [
      base
      (import ../default.nix)
      {
        config.d2b.artifacts = {
          runtime-cloud-hypervisor = {
            package = providerPackage;
            type = "provider";
          };
          guest-system = {
            package = systemPackage;
            type = "nixos-system";
          };
        };
        config.d2b.zones.dev.resources = guestResources;
      }
    ];
  };
  absent = lib.evalModules {
    modules = [
      base
      (import ../default.nix)
      {
        config.d2b.artifacts = {
          runtime-cloud-hypervisor = {
            package = providerPackage;
            type = "provider";
          };
          guest-system = {
            package = systemPackage;
            type = "nixos-system";
          };
        };
        config.d2b.zones.dev.resources.guest = {
          type = "Guest";
          spec = {
            providerRef = "Provider/runtime-cloud-hypervisor";
            systemArtifactId = "guest-system";
          };
        };
      }
    ];
  };
  missingArtifact = lib.evalModules {
    modules = [
      base
      (import ../default.nix)
      {
        config.d2b.artifacts = {
          runtime-cloud-hypervisor = {
            package = providerPackage;
            type = "provider";
          };
        };
        config.d2b.zones.dev.resources = guestResources // {
          guest = guestResources.guest // {
            spec = guestResources.guest.spec // {
              systemArtifactId = "missing-system";
            };
          };
        };
      }
    ];
  };
  wrongArtifactType = lib.evalModules {
    modules = [
      base
      (import ../default.nix)
      {
        config.d2b.artifacts = {
          runtime-cloud-hypervisor = {
            package = providerPackage;
            type = "provider";
          };
        };
        config.d2b.zones.dev.resources = guestResources // {
          guest = guestResources.guest // {
            spec = guestResources.guest.spec // {
              systemArtifactId = "runtime-cloud-hypervisor";
            };
          };
        };
      }
    ];
  };
  invalidDescriptorContract = lib.evalModules {
    modules = [
      base
      (import ../default.nix)
      {
        config.d2b._providerCatalog.entries = [{
          id = "runtime-cloud-hypervisor";
          entry = {
            descriptorDigest = "not-a-digest";
          };
        }];
        config.d2b.artifacts = {
          runtime-cloud-hypervisor = {
            package = providerPackage;
            type = "provider";
          };
          guest-system = {
            package = systemPackage;
            type = "nixos-system";
          };
        };
        config.d2b.zones.dev.resources = guestResources;
      }
    ];
  };
  sameName = lib.evalModules {
    modules = [
      base
      (import ../default.nix)
      {
        config.d2b.artifacts = {
          runtime-cloud-hypervisor = {
            package = providerPackage;
            type = "provider";
          };
          guest-system = {
            package = systemPackage;
            type = "nixos-system";
          };
        };
        config.d2b.zones = {
          alpha.resources = guestResources;
          beta.resources = guestResources;
        };
      }
    ];
  };
  bundle = lib.evalModules {
    modules = [
      base
      bundleBase
      (import ../../../../nixos-modules/resources-zones-processes.nix)
      (import ../../../../nixos-modules/bundle-zones.nix)
      (import ../default.nix)
      {
        config.d2b.artifacts = {
          runtime-cloud-hypervisor = {
            package = providerPackage;
            type = "provider";
          };
          guest-system = {
            package = systemPackage;
            type = "nixos-system";
          };
        };
        config.d2b.zones.dev.resources = guestResources;
      }
    ];
    specialArgs = {
      pkgs = {
        writeText = name: _: builtins.toFile name "";
        runCommand = name: _: _: builtins.toFile name "";
      };
    };
  };
  invalidProvider = lib.evalModules {
    modules = [
      base
      (import ../default.nix)
      {
        config.d2b.zones.dev.resources = {
          runtime-cloud-hypervisor = {
            type = "Provider";
            spec.config = {
              controllerExecutionRef = "Host/missing";
              unsupported = true;
            };
          };
        };
      }
    ];
  };
in
{
  cases = {
    "provider-runtime-cloud-hypervisor/guest-only-descriptor" = {
      expr =
        let
          projection = enabled.config.d2b._resourceCompiler
            .providerProjectionRuntimeCloudHypervisor;
          descriptors = projection.privateArtifact.guestSetupDescriptors or [ ];
        in {
          enabled = projection.enabled;
          hasProcessProjection = projection ? processesByZone;
          hasResourceProjection = projection.resourcesByZone == { };
          hasGuestPatchProjection = projection.guestPatchesByZone == { };
          descriptorCount = lib.length descriptors;
          descriptorGuest = (lib.head descriptors).guest or null;
          descriptorSystemArtifactId =
            (lib.head descriptors).descriptor.systemArtifactId or null;
          descriptorHasPrivateData =
            lib.hasInfix "/nix/store/"
              (builtins.toJSON projection.privateArtifact)
            || lib.hasInfix "\"argv\""
              (builtins.toJSON projection.privateArtifact)
            || lib.hasInfix "\"uid\""
              (builtins.toJSON projection.privateArtifact)
            || lib.hasInfix "\"socket\""
              (builtins.toJSON projection.privateArtifact);
        };
      expected = {
        enabled = true;
        hasProcessProjection = false;
        hasResourceProjection = true;
        hasGuestPatchProjection = true;
        descriptorCount = 1;
        descriptorGuest = "guest";
        descriptorSystemArtifactId = "guest-system";
        descriptorHasPrivateData = false;
      };
    };

    "provider-runtime-cloud-hypervisor/absent-provider" = {
      expr = absent.config.d2b._resourceCompiler
        .providerProjectionRuntimeCloudHypervisor;
      expected = {
        enabled = false;
        resourcesByZone = { };
        guestPatchesByZone = { };
        privateArtifact = {
          schemaVersion = 1;
          providerRef = "Provider/runtime-cloud-hypervisor";
          guestSetupDescriptors = [ ];
        };
      };
    };
    "provider-runtime-cloud-hypervisor/rejects-invalid-provider-settings" = {
      expr = lib.any
        (record: !record.assertion)
        invalidProvider.config.assertions;
      expected = true;
    };
    "provider-runtime-cloud-hypervisor/rejects-missing-system-artifact" = {
      expr = lib.any
        (record:
          !record.assertion
          && lib.hasInfix "systemArtifactId" record.message)
        missingArtifact.config.assertions;
      expected = true;
    };
    "provider-runtime-cloud-hypervisor/rejects-wrong-system-artifact-type" = {
      expr = lib.any
        (record:
          !record.assertion
          && lib.hasInfix "nixos-system" record.message)
        wrongArtifactType.config.assertions;
      expected = true;
    };
    "provider-runtime-cloud-hypervisor/rejects-descriptor-contract-mismatch" = {
      expr = lib.any
        (record:
          !record.assertion
          && lib.hasInfix "descriptor" (lib.toLower record.message))
        invalidDescriptorContract.config.assertions;
      expected = true;
    };
    "provider-runtime-cloud-hypervisor/guest-only-bundle" = {
      expr =
        let
          resources =
            bundle.config.d2b._bundle.zoneResourceBundlesV3.dev.data.resources;
          childRows = lib.filter
            (resource: builtins.elem resource.type [ "Process" "Endpoint" "Volume" ])
            resources;
          guest = lib.findFirst
            (resource:
              resource.type == "Guest" && resource.metadata.name == "guest")
            null
            resources;
          encoded = builtins.toJSON resources;
        in {
          childRows = childRows;
          guestProvider = guest.spec.providerRef;
          guestSystemArtifactId = guest.spec.systemArtifactId;
          noPrivateData =
            !(lib.hasInfix "/nix/store/" encoded)
            && !(lib.hasInfix "\"argv\"" encoded)
            && !(lib.hasInfix "\"uid\"" encoded)
            && !(lib.hasInfix "\"credential\"" encoded)
            && !(lib.hasInfix "\"socket\"" encoded);
        };
      expected = {
        childRows = [ ];
        guestProvider = "Provider/runtime-cloud-hypervisor";
        guestSystemArtifactId = "guest-system";
        noPrivateData = true;
      };
    };
    "provider-runtime-cloud-hypervisor/same-guest-name-is-zone-local" = {
      expr =
        let
          descriptors = sameName.config.d2b._resourceCompiler
            .providerProjectionRuntimeCloudHypervisor.privateArtifact
            .guestSetupDescriptors;
        in {
          zones = map (descriptor: descriptor.zone) descriptors;
          guests = map (descriptor: descriptor.guest) descriptors;
          distinct = (lib.head descriptors) !=
            (lib.elemAt descriptors 1);
        };
      expected = {
        zones = [ "alpha" "beta" ];
        guests = [ "guest" "guest" ];
        distinct = true;
      };
    };
  };
}
