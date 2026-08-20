{ lib, modules, ... }:

let
  module = builtins.head modules;
  evaluated = lib.evalModules {
    modules = [
      { _module.check = false; }
      module
    ];
  };
in
{
  cases = {
    "provider-volume-local/modules-evaluate" = {
      expr = builtins.deepSeq evaluated.config.d2bVolumeLocal true;
      expected = true;
      propagateError = true;
    };

    "volume-local/module-is-a-module" = {
      expr = builtins.isFunction (import ../default.nix);
      expected = true;
    };

    "volume-local/source-policies-default-empty" = {
      expr = evaluated.config.d2bVolumeLocal.sourcePolicies;
      expected = [ ];
    };
  };
}
