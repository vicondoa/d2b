{ lib, modules, ... }:

let
  module = builtins.head modules;
  source = builtins.readFile module;
in
{
  cases = {
    "provider-device-usbip/module-is-a-module" = {
      expr = builtins.isFunction (import module);
      expected = true;
    };

    "provider-device-usbip/guest-vhci-module" = {
      expr = lib.hasInfix "vhci_hcd" source;
      expected = true;
    };
  };
}
