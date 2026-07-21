# Guest-side wiring for hardware video decode via virtio-media.
#
# Uses the chromeos/virtio-media guest driver (device ID 48).
# The dedicated --vhost-user-media CH device type provides proper SHM
# support (same pattern as the GPU device), avoiding the broken
# generic-vhost-user SHM path.
{ config
, lib
, pkgs
, d2bRealmId
, d2bWorkloadId
, d2bRoleIds
, ...
}:

let
  virtioMediaModule = config.boot.kernelPackages.callPackage
    ../../../pkgs/virtio-media-driver { };
  videoSocket =
    "/run/d2b/r/${d2bRealmId}/w/${d2bWorkloadId}/roles/${d2bRoleIds.video}/video.sock";
in
{
  microvm.hypervisor = lib.mkDefault "cloud-hypervisor";

  microvm.cloud-hypervisor.extraArgs = lib.mkAfter [
    "--vhost-user-media"
    "socket=${videoSocket}"
  ];

  boot.extraModulePackages = [ virtioMediaModule ];
  boot.kernelModules = [ "virtio_media" ];
}
