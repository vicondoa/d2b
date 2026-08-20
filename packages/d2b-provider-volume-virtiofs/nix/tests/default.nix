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
    "provider-volume-virtiofs/modules-evaluate" = {
      expr = builtins.deepSeq evaluated.config.d2bVolumeVirtiofs true;
      expected = true;
      propagateError = true;
    };

    "volume-virtiofs/default-cache-is-auto" = {
      expr = evaluated.config.d2bVolumeVirtiofs.cache;
      expected = "auto";
    };
  };
}
