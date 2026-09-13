# Bounded qemu-media Provider defaults for v3 resource declarations.
#
# Guest media, Device, Network, and Volume references remain Zone resources.
# This option group carries only Provider-wide bounded defaults; it never
# accepts paths, argv, bus IDs, credentials, or fd numbers.
{ lib, ... }:

{
  options.d2b.qemuMediaRuntime = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Enable the v3 runtime-qemu-media Provider defaults.";
    };

    kvmRequired = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Require Device/host-kvm for qemu-media Guests.";
    };

    qmpReadyTimeoutSeconds = lib.mkOption {
      type = lib.types.ints.between 5 300;
      default = 30;
      description = "Bounded QMP capability greeting deadline.";
    };

    qmpOperationTimeoutSeconds = lib.mkOption {
      type = lib.types.ints.between 5 300;
      default = 60;
      description = "Bounded QMP command deadline.";
    };

    runtimeTmpfsQuotaBytes = lib.mkOption {
      type = lib.types.ints.between (1024 * 1024) (256 * 1024 * 1024);
      default = 10 * 1024 * 1024;
      description = "Per-Guest runtime tmpfs byte quota.";
    };

    runtimeTmpfsQuotaInodes = lib.mkOption {
      type = lib.types.ints.between 64 65536;
      default = 1024;
      description = "Per-Guest runtime tmpfs inode quota.";
    };
  };
}
