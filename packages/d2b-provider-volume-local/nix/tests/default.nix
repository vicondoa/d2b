{ lib, modules, ... }:

let
  module = builtins.head modules;
  evaluated = lib.evalModules {
    modules = [
      { _module.check = false; }
      ({ lib, ... }: {
        options.d2b.zones = lib.mkOption {
          type = lib.types.attrs;
          default = { };
        };
      })
      module
    ];
  };
in
{
  cases = {
    "provider-volume-local/modules-evaluate" = {
      expr = builtins.deepSeq evaluated.config.d2b.volumes true;
      expected = true;
      propagateError = true;
    };

    "volume-local/module-is-a-module" = {
      expr = builtins.isFunction (import ../default.nix);
      expected = true;
    };

    "volume-local/volume-options-are-declared" = {
      expr = builtins.hasAttr "volumes" evaluated.options.d2b;
      expected = true;
    };
  };
}
