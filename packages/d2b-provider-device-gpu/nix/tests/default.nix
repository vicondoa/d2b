{ lib, modules, pkgs, ... }:

let
  module = builtins.head modules;
  evaluated = lib.evalModules {
    specialArgs = {
      inherit pkgs;
      name = "provider-test";
    };
    modules = [
      ({ lib, ... }: {
        options = {
          microvm.hypervisor = lib.mkOption {
            type = lib.types.str;
            default = "";
          };
          microvm.cloud-hypervisor.extraArgs = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
          };
          boot.extraModulePackages = lib.mkOption {
            type = lib.types.listOf lib.types.package;
            default = [ ];
          };
          boot.kernelModules = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
          };
          boot.kernelPackages = lib.mkOption {
            type = lib.types.raw;
            default = pkgs.linuxPackages;
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
          host-system = { type = "Host"; spec = { }; };
          device-gpu = {
            type = "Provider";
            spec.config.controllerExecutionRef = "Host/host-system";
          };
          guest = { type = "Guest"; spec = { }; };
          gpu = {
            type = "Device";
            metadata.ownerRef = "Guest/guest";
            spec = {
              providerRef = "Provider/device-gpu";
              arbitration = "exclusive";
              provider.settings.videoSidecar = true;
            };
          };
        };
      }
    ];
  };
in
{
  cases = {
    "provider-device-gpu/modules-evaluate" = {
      expr = builtins.deepSeq config.microvm.cloud-hypervisor.extraArgs true;
      expected = true;
      propagateError = true;
    };

    "provider-device-gpu/video-worker-contract" = {
      expr = {
        hypervisor = config.microvm.hypervisor;
        mediaFlag = builtins.elem "--vhost-user-media"
          config.microvm.cloud-hypervisor.extraArgs;
        socket = builtins.elem
          "socket=/run/d2b-video/provider-test/video.sock"
          config.microvm.cloud-hypervisor.extraArgs;
        kernel = builtins.elem "virtio_media" config.boot.kernelModules;
      };
      expected = {
        hypervisor = "cloud-hypervisor";
        mediaFlag = true;
        socket = true;
        kernel = true;
      };
    };
    "provider-device-gpu/projects-gpu-and-video-processes" = {
      expr = lib.attrNames (projected.config.d2b._resourceCompiler
        .providerProjectionDeviceGpu.processesByZone.dev);
      expected = [ "gpu-gpu" "video-gpu" ];
    };
  };
}
