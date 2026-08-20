{ lib, modules, ... }:

let
  module = builtins.head modules;
  source = builtins.readFile module;
in
{
  cases = {
    "provider-device-tpm/module-is-a-module" = {
      expr = builtins.isFunction (import module);
      expected = true;
    };

    "provider-device-tpm/tpm-crb-module" = {
      expr = lib.hasInfix "tpm_crb" source;
      expected = true;
    };
  };
}
