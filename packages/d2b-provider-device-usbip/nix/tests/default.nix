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
          d2b.componentSession.usbipPath = lib.mkOption {
            type = lib.types.str;
            default = "";
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
      expr = {
        kernel = builtins.elem "vhci_hcd" config.boot.kernelModules;
        tools = config.environment.systemPackages != [ ];
        controlPath = config.d2b.componentSession.usbipPath != "";
      };
      expected = {
        kernel = true;
        tools = true;
        controlPath = true;
      };
    };
  };
}
