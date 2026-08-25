{ lib, modules, pkgs, ... }:

let
  module = builtins.head modules;
  evaluated = lib.evalModules {
    specialArgs = { inherit pkgs; };
    modules = [
      ({ lib, ... }: {
        options = {
          boot.kernelModules = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
          };
          environment.systemPackages = lib.mkOption {
            type = lib.types.listOf lib.types.package;
            default = [ ];
          };
        };
      })
      module
    ];
  };
  config = evaluated.config;
in
{
  cases = {
    "provider-device-usbip/modules-evaluate" = {
      expr = builtins.deepSeq config.boot.kernelModules true;
      expected = true;
      propagateError = true;
    };

    "provider-device-usbip/guest-vhci-module" = {
      # Guest import is owned by the signed USBIP Provider Process over
      # ComponentSession; this module renders only the kernel/tool inputs.
      expr = {
        kernel = builtins.elem "vhci_hcd" config.boot.kernelModules;
        tools = config.environment.systemPackages != [ ];
      };
      expected = {
        kernel = true;
        tools = true;
      };
    };
  };
}
