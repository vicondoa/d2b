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
  evaluated = lib.evalModules {
    modules = [
      base
      (import ../default.nix)
      {
        config.d2b.zones.dev.resources = {
          runtime-azure-virtual-machine = {
            type = "Provider";
            spec.config.controllerExecutionRef = "Guest/gateway";
          };
          gateway = { type = "Guest"; spec = { }; };
        };
      }
    ];
  };
in
{
  cases = {
    "provider-runtime-azure-virtual-machine/gateway-process" = {
      expr = evaluated.config.d2b._resourceCompiler
        .providerProjectionRuntimeAzureVirtualMachine.processesByZone.dev
        ."azure-vm-controller".spec.template;
      expected = "azure-vm-controller";
    };
    "provider-runtime-azure-virtual-machine/process-stays-in-gateway" = {
      expr = evaluated.config.d2b._resourceCompiler
        .providerProjectionRuntimeAzureVirtualMachine.processesByZone.dev
        ."azure-vm-controller".spec.executionRef;
      expected = "Guest/gateway";
    };
  };
}
