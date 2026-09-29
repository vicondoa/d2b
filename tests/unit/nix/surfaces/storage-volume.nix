{ lib, pkgs, system, nixpkgs, inputs, d2bModule, d2bLib, flakeRoot, modules }:

import ../helpers/surface.nix {
  inherit lib pkgs system nixpkgs inputs d2bModule d2bLib flakeRoot modules;
  name = "storage-volume";
  caseFiles = [
    {
      path = ../cases/volume-mounts.nix;
      names = [
        "volume-mounts/serial-null-defaults"
        "volume-mounts/v3-attachment-stays-declared-binding-input"
        "volume-mounts/v3-attachment-emits-no-durable-binding-resource"
        "volume-mounts/v3-attachment-emits-binding-worker-principal"
        "volume-mounts/v3-binding-worker-principal-follows-attachment"
        "volume-mounts/v3-acl-grant-wider-than-the-group-class-is-refused"
        "volume-mounts/v3-acl-grants-inside-the-group-class-pass"
        "volume-mounts/virtiofs-attached-guest-declares-vm-run-posture"
        "volume-mounts/virtiofs-attached-guest-declares-no-swtpm-state-row"
        "volume-mounts/device-and-virtiofs-guest-emits-one-vm-run-row"
        "volume-mounts/device-owner-keeps-the-vm-run-posture"
        "volume-mounts/virtio-blk-attached-guest-declares-no-vm-run-row"
        "volume-mounts/host-targeted-attachment-declares-no-vm-run-row"
      ];
    }
  ];
}
