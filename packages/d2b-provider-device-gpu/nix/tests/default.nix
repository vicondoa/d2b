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
          gpu-nvidia = {
            type = "Device";
            metadata.ownerRef = "Guest/guest";
            spec = {
              providerRef = "Provider/device-gpu";
              arbitration = "exclusive";
              provider.settings = {
                videoSidecar = true;
                videoNvidiaDecode = true;
              };
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
      expected = [ "gpu-gpu" "gpu-gpu-nvidia" "video-gpu" "video-gpu-nvidia" ];
    };
    "provider-device-gpu/worker-posture" = {
      expr = let
        processes = projected.config.d2b._resourceCompiler
          .providerProjectionDeviceGpu.processesByZone.dev;
        gpu = processes.gpu-gpu;
        video = processes.video-gpu;
      in {
        gpuOwner = gpu.metadata.ownerRef;
        gpuTemplate = gpu.spec.template;
        gpuSandbox = gpu.spec.sandbox;
        gpuRestartPolicy = gpu.spec.restartPolicy;
        videoOwner = video.metadata.ownerRef;
        videoTemplate = video.spec.template;
        videoSandbox = video.spec.sandbox;
        videoRestartPolicy = video.spec.restartPolicy;
      };
      expected = {
        gpuOwner = "Device/gpu";
        gpuTemplate = "gpu-worker";
        # A persistent launch refusal (a host-device grant the broker
        # refuses, a site with no projected Wayland socket) must reach the
        # closed terminal classification instead of retrying forever. The
        # ceiling is a per-daemon-lifetime launch counter: the driver's
        # in-memory budget never resets, so `resetAfter` has no effect.
        gpuRestartPolicy = {
          class = "on-failure";
          backoffBase = "1s";
          backoffMax = "60s";
          backoffMultiplierMilli = 2000;
          maxRestarts = 2;
          resetAfter = "300s";
        };
        gpuSandbox = {
          namespaceClasses = [ "mount" "pid" "ipc" "uts" "user" ];
          capabilityClasses = [ ];
          seccompClass = "strict";
          noNewPrivileges = true;
          startRoot = false;
          readOnlyRoot = true;
          environmentClass = "minimal";
          umask = "0007";
          oomScoreAdj = 0;
          userNamespace = { mappingClass = "process-principal-root"; };
        };
        videoOwner = "Device/gpu";
        videoTemplate = "video-worker";
        videoRestartPolicy = {
          class = "on-failure";
          backoffBase = "1s";
          backoffMax = "60s";
          backoffMultiplierMilli = 2000;
          maxRestarts = 2;
          resetAfter = "300s";
        };
        # The video sidecar fences a pid namespace and a DRI device bind, and
        # never a user namespace.
        videoSandbox = {
          namespaceClasses = [ "mount" "pid" "ipc" "uts" ];
          capabilityClasses = [ ];
          seccompClass = "strict";
          noNewPrivileges = true;
          startRoot = false;
          readOnlyRoot = true;
          environmentClass = "minimal";
          umask = "0007";
          oomScoreAdj = 0;
          userNamespace = null;
        };
      };
    };
    "provider-device-gpu/nvidia-video-template" = {
      expr = let
        processes = projected.config.d2b._resourceCompiler
          .providerProjectionDeviceGpu.processesByZone.dev;
        plain = processes.video-gpu;
        nvidia = processes.video-gpu-nvidia;
      in {
        inherit (nvidia.spec) template deviceUsage;
        # The resource compiler fences a row's sandbox against its template's
        # closed posture; both video templates share the one video sandbox.
        sameSandbox = nvidia.spec.sandbox == plain.spec.sandbox;
        sameRestartPolicy = nvidia.spec.restartPolicy == plain.spec.restartPolicy;
      };
      expected = {
        template = "video-worker-nvidia";
        deviceUsage = [{
          deviceRef = "Device/gpu-nvidia";
          access = "shared";
          purpose = "video-decode";
        }];
        sameSandbox = true;
        sameRestartPolicy = true;
      };
    };
  };
}
