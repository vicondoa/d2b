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
      default = { };
      internal = true;
      visible = false;
    };
  };
  enabled = lib.evalModules {
    modules = [
      base
      (import ../default.nix)
      {
        config.d2b.zones.dev.resources = {
          host-system = { type = "Host"; spec = { }; };
          runtime-cloud-hypervisor = {
            type = "Provider";
            spec.config.controllerExecutionRef = "Host/host-system";
          };
          guest = {
            type = "Guest";
            spec.providerRef = "Provider/runtime-cloud-hypervisor";
          };
        };
      }
    ];
  };
  absent = lib.evalModules {
    modules = [
      base
      (import ../default.nix)
      {
        config.d2b.zones.dev.resources.guest = {
          type = "Guest";
          spec.providerRef = "Provider/runtime-cloud-hypervisor";
        };
      }
    ];
  };
  invalid = lib.evalModules {
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
    "provider-runtime-cloud-hypervisor/guest-process" = {
      expr = enabled.config.d2b._resourceCompiler
        .providerProjectionRuntimeCloudHypervisor.processesByZone.dev
        ."cloud-hypervisor-guest".spec.template;
      expected = "cloud-hypervisor-runner";
    };

    "provider-runtime-cloud-hypervisor/absent-provider" = {
      expr = absent.config.d2b._resourceCompiler
        .providerProjectionRuntimeCloudHypervisor.processesByZone.dev or { };
      expected = { };
    };
    "provider-runtime-cloud-hypervisor/rejects-invalid-provider-settings" = {
      expr = lib.any
        (record: !record.assertion)
        invalid.config.assertions;
      expected = true;
    };
  };
}
