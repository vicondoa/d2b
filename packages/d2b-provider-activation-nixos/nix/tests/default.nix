{ lib, modules, ... }:

let
  module = builtins.head modules;
  evaluated = lib.evalModules {
    modules = [
      { _module.check = false; }
      module
    ];
  };
  projected = lib.evalModules {
    modules = [
      {
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
      }
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources = {
          activation-nixos = { type = "Provider"; spec = { }; };
          guest = {
            type = "Guest";
            spec.systemArtifactId = "guest-system";
          };
        };
      }
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
    "activation-nixos/projects-guest-generation" = {
      expr = {
        generation = projected.config.d2b._resourceCompiler
          .providerProjectionActivationNixos.resourcesByZone.dev
          ."activation-guest".spec;
        process = projected.config.d2b._resourceCompiler
          .providerProjectionActivationNixos.processesByZone.dev
          ."activation-runner-activation-guest".spec.template;
      };
      expected = {
        generation = {
          providerRef = "Provider/activation-nixos";
          executionRef = "Guest/guest";
          systemArtifactId = "guest-system";
          activationMode = "switch";
        };
        process = "activation-nixos-runner";
      };
    };
  };
}
