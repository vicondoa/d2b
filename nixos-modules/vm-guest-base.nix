# nixos-modules/vm-guest-base.nix
#
# d2b-owned per-VM guest-side baseline. Replaces the
# guest-side portion of `microvm.nix`'s
# `microvm.nixosModules.microvm` (the per-VM eval module that
# upstream `microvm.nixosModules.host` injected into every VM's
# evaluation).
#
# This module is added by `vm-evaluator.nix` to every per-VM eval
# and provides the substrate that lets `config.system.build.toplevel`
# succeed without microvm.nix:
#
#   1. Disable bootloader (no grub on a microvm).
#   2. Provide a tmpfs root + bind-mount /nix/store from the host
#      share + mount every other virtiofs/9p share at its
#      `mountPoint`.
#   3. Pass `init=${toplevel}/init` via the kernel cmdline so the
#      hypervisor's `-append` arg can boot straight into the
#      system closure.
#   4. Load the virtio kernel modules at initrd time.
#
# Modeled after upstream microvm.nix's `microvm/system.nix` +
# `microvm/mounts.nix` (see
# `/nix/store/.../microvm.nix-source/nixos-modules/microvm/`)
# but stripped to only the bits d2b's broker SpawnRunner
# substrate actually needs. The upstream module pulled in
# `microvm.runner`, `boot-disk`, `store-disk`, `interfaces`,
# `pci-devices`, `virtiofsd`, `graphics`, `rosetta`, `ssh-deploy`,
# `vsock-ssh` — none of those are needed for the daemon-native
# substrate because the broker owns hypervisor argv generation
# (`packages/d2b-host/src/*_argv.rs`) and runtime supervision.
{ config, lib, pkgs, ... }:

let
  cfg = config.microvm;
  d2bLib = import ./lib.nix { inherit lib; };

  # Find the host-store share (source == "/nix/store") — required
  # to be present by `store.nix` / `host.nix`'s composeVm pass; it
  # produces a virtiofs share with tag e.g. "store" mounted at
  # `/nix/.ro-store` (or whatever the consumer picked) so the
  # writable-overlay can layer on top.
  hostStoreShares = builtins.filter (s: s.source == "/nix/store") cfg.shares;
  hostStore = if hostStoreShares == [ ] then null else builtins.head hostStoreShares;

  # When writableStoreOverlay is set, the read-only lower is the
  # host-store share mount point; the writable upper lives at
  # `${overlay}/store` and the workdir at `${overlay}/work`.
  hasOverlay = cfg.writableStoreOverlay != null;

  volumeFileSystems = builtins.listToAttrs (map (volume: {
    name = volume.mountPoint;
    value = d2bLib.volumeFileSystem volume;
  }) cfg.volumes);
in

{
  config = {
    # No grub on a microvm.
    boot.loader.grub.enable = lib.mkDefault false;
    boot.loader.systemd-boot.enable = lib.mkDefault false;

    # virtio drivers must be in initrd so the rootfs + nix-store
    # share can be mounted.
    boot.initrd.kernelModules = [
      "virtio_mmio"
      "virtio_pci"
      "virtio_blk"
      "virtio_net"
      "virtio_console"
      "virtiofs"
      "9pnet_virtio"
      "9p"
    ] ++ lib.optional hasOverlay "overlay";

    # Kernel cmdline: boot straight into the system closure.
    microvm.kernelParams =
      let
        toplevel =
          if cfg.storeOnDisk
          then builtins.unsafeDiscardStringContext config.system.build.toplevel
          else config.system.build.toplevel;
      in
        config.boot.kernelParams ++ [ "init=${toplevel}/init" ];

    # rfkill / intel_pstate are useless in a microvm and slow boot.
    # drm only matters if graphics is enabled.
    boot.blacklistedKernelModules = [ "rfkill" "intel_pstate" ]
      ++ lib.optional (!cfg.graphics.enable) "drm";

    # Disable services that the upstream microvm module disables
    # because they hang / break in the microvm context.
    systemd.services.mount-pstore.enable = false;
    systemd.generators.systemd-gpt-auto-generator = "/dev/null";

    fileSystems = lib.mkMerge [
      # tmpfs / root (consumer can override).
      {
        "/" = lib.mkDefault {
          device = "rootfs";
          fsType = "tmpfs";
          options = [ "size=50%,mode=0755" ];
          neededForBoot = true;
        };
      }

      # /nix/store: either bind-mount from the host-store share (no
      # overlay), or layered overlay (writable upper + read-only lower).
      # mount /nix/store DIRECTLY from the ro-store
      # virtiofs tag instead of bind-from-/nix/.ro-store. This
      # avoids a kernel quirk where bind-mounting a virtiofs onto
      # /nix/store loses exec permissions or sees an empty
      # directory under the broker-spawn model. The hostStore
      # share's mount-point /nix/.ro-store still works for
      # consumers that opt into the writable-store-overlay path.
      (lib.optionalAttrs (hostStore != null && !hasOverlay) {
        "/nix/store" = {
          device = hostStore.tag;
          fsType = hostStore.proto;
          options = [ "ro" "x-initrd.mount" "x-systemd.after=systemd-modules-load.service" ];
          neededForBoot = true;
        };
      })

      # when overlay IS enabled, the ro-store virtiofs
      # share MUST still be mounted as the overlayfs lowerdir
      # source.  The foldl' below skips it (`s.source == "/nix/store"`
      # is its skip predicate), so without this explicit mount the
      # lowerdir is just an empty directory on the rootfs tmpfs and
      # the overlayfs mount hangs in initramfs waiting for it.
      # neededForBoot ensures the mount happens in initrd before
      # the overlay block tries to assemble the layered /nix/store.
      (lib.optionalAttrs (hostStore != null && hasOverlay) {
        "${hostStore.mountPoint}" = {
          device = hostStore.tag;
          fsType = hostStore.proto;
          options = [ "ro" "x-initrd.mount" "x-systemd.after=systemd-modules-load.service" ];
          neededForBoot = true;
        };
      })

      # writableStoreOverlay backing-disk mount.
      # The broker attaches store-overlay.img to CH with serial=rootfs
      # (see nixos-modules/processes-json.nix), so the guest sees
      # /dev/disk/by-id/virtio-rootfs.  Mount it at the overlay path
      # so the upperdir/workdir live on a real filesystem (ext4 — the
      # broker's DiskInit op runs mkfs.ext4 when creating the image;
      # see packages/d2b-priv-broker/src/ops/disk_init.rs).
      # Without this mount the overlay upper/work live on the rootfs
      # tmpfs and are wiped on every reboot, which defeats the
      # writableStoreOverlay design.
      (lib.optionalAttrs hasOverlay {
        "${cfg.writableStoreOverlay}" = {
          device = "/dev/disk/by-id/virtio-rootfs";
          fsType = "ext4";
          options = [ "x-initrd.mount" "x-systemd.after=systemd-modules-load.service" ];
          neededForBoot = true;
        };
      })

      (lib.optionalAttrs hasOverlay {
        "/nix/store" = {
          neededForBoot = true;
          overlay = {
            lowerdir = [ (if hostStore != null then hostStore.mountPoint else "/nix/.ro-store") ];
            upperdir = "${cfg.writableStoreOverlay}/store";
            workdir = "${cfg.writableStoreOverlay}/work";
          };
        };
      })

      # Per-VM block volumes declared through the preserved
      # `microvm.volumes` option. processes-json.nix emits the same
      # default virtio serial for each disk, so the guest can mount by
      # stable /dev/disk/by-id path instead of ephemeral vda/vdb order.
      volumeFileSystems

      # All other virtiofs/9p shares are mounted at their
      # `mountPoint`. Skip the host-store share (handled above) and
      # any share whose mountPoint is the overlay path (handled by
      # the overlay block).
      (builtins.foldl'
        (acc: s:
          acc // (
            # skip ro-store entirely - mounted directly
            # at /nix/store via the dedicated fileSystems entry above
            # (no-overlay case) or as overlay lowerdir (overlay case).
            if s.source == "/nix/store"
               || (hasOverlay && s.mountPoint == cfg.writableStoreOverlay)
            then { }
            else {
              "${s.mountPoint}" = {
                device = s.tag;
                fsType = s.proto;
                options = {
                  virtiofs = [ "defaults" "x-systemd.after=systemd-modules-load.service" ];
                  "9p" = [ "trans=virtio" "version=9p2000.L" "msize=65536"
                           "x-systemd.after=systemd-modules-load.service" ];
                }.${s.proto};
              };
            }
          )
        )
        { }
        cfg.shares)
    ];
  };
}
