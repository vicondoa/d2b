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
  };
}
