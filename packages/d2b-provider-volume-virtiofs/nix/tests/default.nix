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
      # No Guest-owned Process row is projected: the Process driver refuses a
      # Guest-owned row that is not `<guest>-vmm`, so only the store preflight
      # intent remains (the binding driver mints the live virtiofsd worker).
      expr = {
        rows = builtins.attrNames evaluated.config.d2b._resourceCompiler
          .providerProjectionVolumeVirtiofs.processesByZone.dev;
        preflight = evaluated.config.d2b._resourceCompiler
          .providerProjectionVolumeVirtiofs.processesByZone.dev
          ."store-preflight-guest".spec.template;
      };
      expected = {
        rows = [ "store-preflight-guest" ];
        preflight = "store-virtiofs-preflight";
      };
    };

    "provider-volume-virtiofs/module-is-a-module" = {
      expr = builtins.isFunction (import ../default.nix);
      expected = true;
    };
  };
}
