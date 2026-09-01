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
      (import ../default.nix)
      {
        config.d2b.zones.dev.resources = {
          device-usbip = { type = "Provider"; spec = { }; };
          guest = { type = "Guest"; spec = { }; };
          usb-binding = {
            type = "usb.d2bus.org.UsbBinding";
            spec = {
              providerRef = "Provider/device-usbip";
              guestRef = "Guest/guest";
            };
          };
        };
      }
    ];
  };
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
    "provider-device-usbip/projects-guest-process-and-endpoint" = {
      expr = {
        processes = lib.attrNames (projected.config.d2b._resourceCompiler
          .providerProjectionDeviceUsbip.processesByZone.dev);
        endpoints = lib.attrNames (projected.config.d2b._resourceCompiler
          .providerProjectionDeviceUsbip.resourcesByZone.dev);
      };
      expected = {
        processes = [ "usbip-usb-binding" ];
        endpoints = [ "usbip-usb-binding" ];
      };
    };
  };
}
