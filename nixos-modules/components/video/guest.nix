# Guest-side wiring for hardware video decode via virtio-media.
#
# Uses the chromeos/virtio-media guest driver (device ID 48).
# The dedicated --vhost-user-media CH device type provides proper SHM
# support (same pattern as the GPU device), avoiding the broken
# generic-vhost-user SHM path.
{ config, lib, pkgs, name, ... }:

let
  vmName = name;
  virtioMediaModule = config.boot.kernelPackages.callPackage
    ../../../pkgs/virtio-media-driver { };
in
{
  microvm.hypervisor = lib.mkDefault "cloud-hypervisor";

  microvm.cloud-hypervisor.extraArgs = lib.mkAfter [
    "--vhost-user-media"
    "socket=/run/nixling-video/${vmName}/video.sock"
  ];

  boot.extraModulePackages = [ virtioMediaModule ];
  boot.kernelModules = [ "virtio_media" ];
}
