{ lib, ... }:

let
  base = {
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
          host-system = { type = "Host"; spec = { }; };
          volume-virtiofs = {
            type = "Provider";
            spec.config.controllerExecutionRef = "Host/host-system";
          };
          guest = { type = "Guest"; spec = { }; };
          state = {
            type = "Volume";
            spec = {
              attachments = [{
                executionRef = "Guest/guest";
                transport = "virtiofs";
              }];
            };
          };
        };
      }
    ];
  };
in
{
  cases = {
    "provider-volume-virtiofs/guest-process" = {
      expr = {
        process = evaluated.config.d2b._resourceCompiler
          .providerProjectionVolumeVirtiofs.processesByZone.dev
          ."virtiofsd-guest".spec.template;
        preflight = evaluated.config.d2b._resourceCompiler
          .providerProjectionVolumeVirtiofs.processesByZone.dev
          ."store-preflight-guest".spec.template;
      };
      expected = {
        process = "virtiofsd";
        preflight = "store-virtiofs-preflight";
      };
    };

    "provider-volume-virtiofs/module-is-a-module" = {
      expr = builtins.isFunction (import ../default.nix);
      expected = true;
    };
  };
}
