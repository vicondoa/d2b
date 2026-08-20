{ lib, modules, ... }:

let
  module = builtins.head modules;
  source = builtins.readFile module;
in
{
  cases = {
    "provider-device-security-key/module-is-a-module" = {
      expr = builtins.isFunction (import module);
      expected = true;
    };

    "provider-device-security-key/guest-frontend-contract" = {
      expr =
        builtins.all
          (needle: lib.hasInfix needle source)
          [ "d2b-sk-frontend" "D2B_SK_VSOCK_PORT" "NoNewPrivileges" ];
      expected = true;
    };
  };
}
