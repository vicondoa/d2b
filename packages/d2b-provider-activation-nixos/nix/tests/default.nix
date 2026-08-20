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
    "provider-activation-nixos/modules-evaluate" = {
      expr = builtins.deepSeq evaluated.config.d2b.providers.activationNixos true;
      expected = true;
      propagateError = true;
    };

    "activation-nixos/default-retention-is-bounded" = {
      expr = evaluated.config.d2b.providers.activationNixos.retainedGenerations;
      expected = 3;
    };
  };
}
