{ lib, modules, ... }:

let
  module = builtins.head modules;
  evaluated = lib.evalModules {
    modules = [
      {
        _module.check = false;
      }
      ({ lib, ... }: {
        options.networking.networkmanager.unmanaged = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
        };
      })
      module
    ];
  };
in
{
  cases = {
    "provider-network-local/modules-evaluate" = {
      expr =
        builtins.deepSeq
          evaluated.config.networking.networkmanager.unmanaged
          true;
      expected = true;
      propagateError = true;
    };

    "network-local/module-is-a-module" = {
      expr = builtins.isFunction (import ../default.nix);
      expected = true;
    };

    "network-local/module-defines-networkmanager-unmanaged" = {
      expr =
        let value = (import ../default.nix { inherit lib; });
        in value.networking.networkmanager.unmanaged._type;
      expected = "order";
    };
  };
}
