{ lib, modules, pkgs, ... }:

let
  module = builtins.head modules;
  evaluated = lib.evalModules {
    specialArgs = { inherit pkgs; };
    modules = [
      ({ lib, ... }: {
        options = {
          networking.hostName = lib.mkOption {
            type = lib.types.str;
            default = "provider-test";
          };
          microvm.hypervisor = lib.mkOption {
            type = lib.types.str;
            default = "";
          };
          microvm.cloud-hypervisor.extraArgs = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
          };
          security.tpm2.enable = lib.mkOption {
            type = lib.types.bool;
            default = false;
          };
          boot.kernelModules = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
          };
          environment.systemPackages = lib.mkOption {
            type = lib.types.listOf lib.types.package;
            default = [ ];
          };
          systemd.services = lib.mkOption {
            type = lib.types.attrsOf lib.types.anything;
            default = { };
          };
        };
      })
      module
    ];
  };
  config = evaluated.config;
  projected = lib.evalModules {
    modules = [
      {
        options.d2b.zones = lib.mkOption {
          type = lib.types.attrs;
          default = { };
        };
        options.d2b._resourceCompiler = lib.mkOption {
          type = lib.types.attrs;
          default = { };
          internal = true;
          visible = false;
        };
      }
      (import ../default.nix)
      {
        config.d2b.zones.dev.resources = {
          host-system = { type = "Host"; spec = { }; };
          device-tpm = {
            type = "Provider";
            spec.config.controllerExecutionRef = "Host/host-system";
          };
          guest = { type = "Guest"; spec = { }; };
          tpm = {
            type = "Device";
            metadata.ownerRef = "Guest/guest";
            spec.providerRef = "Provider/device-tpm";
          };
        };
      }
    ];
  };
  incompleteProvider = lib.evalModules {
    modules = [
      {
        options.d2b.zones = lib.mkOption {
          type = lib.types.attrs;
          default = { };
        };
        options.d2b._resourceCompiler = lib.mkOption {
          type = lib.types.attrs;
          default = { };
          internal = true;
          visible = false;
        };
      }
      (import ../default.nix)
      {
        config.d2b.zones.dev.resources = {
          device-tpm = {
            type = "Provider";
            spec = { };
          };
          guest = { type = "Guest"; spec = { }; };
          tpm = {
            type = "Device";
            metadata.ownerRef = "Guest/guest";
            spec.providerRef = "Provider/device-tpm";
          };
        };
      }
    ];
  };
  unresolvedController = lib.evalModules {
    modules = [
      {
        options.d2b.zones = lib.mkOption {
          type = lib.types.attrs;
          default = { };
        };
        options.d2b._resourceCompiler = lib.mkOption {
          type = lib.types.attrs;
          default = { };
          internal = true;
          visible = false;
        };
      }
      (import ../default.nix)
      {
        config.d2b.zones.dev.resources = {
          device-tpm = {
            type = "Provider";
            spec.config.controllerExecutionRef = "Host/missing";
          };
          guest = { type = "Guest"; spec = { }; };
          tpm = {
            type = "Device";
            metadata.ownerRef = "Guest/guest";
            spec.providerRef = "Provider/device-tpm";
          };
        };
      }
    ];
  };
in
{
  cases = {
    "provider-device-tpm/modules-evaluate" = {
      expr = builtins.deepSeq config.boot.kernelModules true;
      expected = true;
      propagateError = true;
    };

    "provider-device-tpm/tpm-crb-module" = {
      expr = {
        tpm = builtins.elem "tpm" config.boot.kernelModules;
        crb = builtins.elem "tpm_crb" config.boot.kernelModules;
        hypervisor = config.microvm.hypervisor;
        tpm2 = config.security.tpm2.enable;
      };
      expected = {
        tpm = true;
        crb = true;
        hypervisor = "cloud-hypervisor";
        tpm2 = true;
      };
    };
    "provider-device-tpm/projects-swtpm-children" = {
      expr = {
        enabled = projected.config.d2b._resourceCompiler
          .providerProjectionDeviceTpm.enabled;
        processes = lib.attrNames (projected.config.d2b._resourceCompiler
          .providerProjectionDeviceTpm.processesByZone.dev);
        endpoints = lib.attrNames (projected.config.d2b._resourceCompiler
          .providerProjectionDeviceTpm.resourcesByZone.dev);
        processRefs = projected.config.d2b._resourceCompiler
          .providerProjectionDeviceTpm.privateArtifact.processRefs;
      };
      expected = {
        enabled = true;
        processes = [ "swtpm-flush-tpm" "swtpm-tpm" ];
        endpoints = [ "tpm-ctrl-tpm" "tpm-tpm" ];
        processRefs = [
          "Process/swtpm-tpm"
          "EphemeralProcess/swtpm-flush-tpm"
        ];
      };
    };

    "provider-device-tpm/present-incomplete-provider-emits-no-projection" = {
      expr = let
        projection = incompleteProvider.config.d2b._resourceCompiler
          .providerProjectionDeviceTpm;
      in {
        enabled = projection.enabled;
        processes = projection.processesByZone.dev or { };
        resources = projection.resourcesByZone.dev;
        processRefs = projection.privateArtifact.processRefs;
        endpointRefs = projection.privateArtifact.endpointRefs;
        guestPatches = projection.guestPatchesByZone;
      };
      expected = {
        enabled = false;
        processes = { };
        resources = { };
        processRefs = [ ];
        endpointRefs = [ ];
        guestPatches = { };
      };
    };

    "provider-device-tpm/endpoints-have-live-process-producers" = {
      expr = let
        projection = projected.config.d2b._resourceCompiler
          .providerProjectionDeviceTpm;
        processRefs = map
          (process: "${process.type}/${process.metadata.name}")
          (lib.attrValues projection.processesByZone.dev);
      in lib.all
        (endpoint: builtins.elem endpoint.spec.producerRef processRefs)
        (lib.attrValues projection.resourcesByZone.dev);
      expected = true;
    };

    "provider-device-tpm/unresolved-controller-emits-no-projection" = {
      expr = let
        projection = unresolvedController.config.d2b._resourceCompiler
          .providerProjectionDeviceTpm;
      in {
        enabled = projection.enabled;
        processes = projection.processesByZone or { };
        resources = projection.resourcesByZone.dev;
      };
      expected = {
        enabled = false;
        processes = { };
        resources = { };
      };
    };
  };
}
