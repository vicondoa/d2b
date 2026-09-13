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
          runtime-azure-container-apps = {
            type = "Provider";
            spec.config.gatewayExecutionRef = "Guest/gateway";
          };
          gateway = { type = "Guest"; spec = { }; };
        };
      }
    ];
  };
in
{
  cases = {
    "provider-runtime-azure-container-apps/gateway-processes" = {
      expr = lib.attrNames (evaluated.config.d2b._resourceCompiler
        .providerProjectionRuntimeAzureContainerApps.processesByZone.dev);
      expected = [ "aca-controller" "aca-deployment-service" ];
    };
    "provider-runtime-azure-container-apps/processes-stay-in-gateway" = {
      expr = evaluated.config.d2b._resourceCompiler
        .providerProjectionRuntimeAzureContainerApps.processesByZone.dev
        ."aca-controller".spec.executionRef;
      expected = "Guest/gateway";
    };
  };
}
