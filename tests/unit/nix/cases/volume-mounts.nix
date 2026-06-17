# nix-unit cases migrated from tests/volume-mounts-eval.sh (group D).
#
# Asserts the shared `nixos-modules/lib.nix` volume helpers
# (volumeSerial / volumeHostPath / volumeFileSystem / volumeSizeBytes /
# volumeDiskInitEligible / volumeSerialIssues) — Cloud Hypervisor disk
# serials, guest fileSystems mounts, MiB->bytes, DiskInit eligibility, and
# the duplicate/reserved/overlong/unsafe serial issue sets.
#
# The "module callsites use the shared helpers" grep checks the bash gate
# also carried are NOT value assertions; they migrate to the hermetic
# flake.checks.<sys>.module-helper-wiring derivation (see flake.nix).
{ nl, ... }:

let
  varVolume = {
    image = "var.img";
    mountPoint = "/var";
    size = 1024;
    fsType = "ext4";
    serial = null;
  };
  externalVolume = {
    image = "/tmp/external.img";
    mountPoint = "/mnt/external";
    size = 1;
    fsType = "ext4";
  };
  nonExt4Volume = {
    image = "data.img";
    mountPoint = "/data";
    size = 1;
    fsType = "xfs";
  };
  qcowVolume = {
    image = "qcow.img";
    mountPoint = "/qcow";
    size = 1;
    fsType = "ext4";
    imageType = "qcow2";
  };

  issues = nl.volumeSerialIssues [
    { image = "var.img"; }
    { image = "var.img"; }
    { image = "rootfs.img"; }
    { image = "this-name-is-definitely-too-long.img"; }
    { image = "ok.img"; serial = "bad,serial"; }
    { image = "ok2.img"; serial = "bad=serial"; }
    { image = "empty.img"; serial = ""; }
  ];

  fs = nl.volumeFileSystem varVolume;
in
{
  "volume-mounts/serial-null-defaults" = {
    expr = nl.volumeSerial varVolume;
    expected = "var";
  };
  "volume-mounts/serial-sanitizes-delimiters" = {
    expr = nl.volumeSerial { image = "bad,name=still.img"; };
    expected = "bad-name-still";
  };
  "volume-mounts/host-path-relative" = {
    expr = nl.volumeHostPath "/var/lib/nixling/vms" "work" varVolume;
    expected = "/var/lib/nixling/vms/work/var.img";
  };
  "volume-mounts/host-path-absolute" = {
    expr = nl.volumeHostPath "/var/lib/nixling/vms" "work" externalVolume;
    expected = "/tmp/external.img";
  };
  "volume-mounts/fs-device" = {
    expr = fs.device;
    expected = "/dev/disk/by-id/virtio-var";
  };
  "volume-mounts/fs-fstype" = {
    expr = fs.fsType;
    expected = "ext4";
  };
  "volume-mounts/fs-needed-for-boot" = {
    expr = fs.neededForBoot;
    expected = true;
  };
  "volume-mounts/fs-options-waits-modules" = {
    expr = builtins.elem "x-systemd.after=systemd-modules-load.service" fs.options;
    expected = true;
  };
  "volume-mounts/size-bytes" = {
    expr = nl.volumeSizeBytes varVolume;
    expected = 1073741824;
  };
  "volume-mounts/disk-init-relative-ext4-raw" = {
    expr = nl.volumeDiskInitEligible varVolume;
    expected = true;
  };
  "volume-mounts/disk-init-absolute" = {
    expr = nl.volumeDiskInitEligible externalVolume;
    expected = false;
  };
  "volume-mounts/disk-init-non-ext4" = {
    expr = nl.volumeDiskInitEligible nonExt4Volume;
    expected = false;
  };
  "volume-mounts/disk-init-non-raw" = {
    expr = nl.volumeDiskInitEligible qcowVolume;
    expected = false;
  };
  "volume-mounts/issues-duplicates" = {
    expr = builtins.elem "var" issues.duplicates;
    expected = true;
  };
  "volume-mounts/issues-reserved" = {
    expr = builtins.elem "rootfs" issues.reserved;
    expected = true;
  };
  "volume-mounts/issues-too-long" = {
    expr = builtins.elem "this-name-is-definitely-too-long" issues.tooLong;
    expected = true;
  };
  "volume-mounts/issues-unsafe-comma" = {
    expr = builtins.elem "bad,serial" issues.unsafe;
    expected = true;
  };
  "volume-mounts/issues-unsafe-equals" = {
    expr = builtins.elem "bad=serial" issues.unsafe;
    expected = true;
  };
  "volume-mounts/issues-unsafe-empty" = {
    expr = builtins.elem "" issues.unsafe;
    expected = true;
  };
}
