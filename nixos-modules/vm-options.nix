# nixos-modules/vm-options.nix
#
# D2b-owned per-VM runner options module. Replaces the upstream
# microvm.nix `microvm.*` per-VM option set.
#
# This module is added to each per-VM NixOS evaluation by
# `vm-evaluator.nix` so consumers' guest configs can set
# `microvm.mem`, `microvm.vcpu`, `microvm.shares`, etc. with the
# same shape they used under microvm.nix (backward-compatible
# option paths inside the per-VM evaluation, but no upstream
# microvm.nix dependency at the flake-input level).
#
# The fields enumerated here are the subset consumed by
# `nixos-modules/processes-json.nix` (via the
# `d2bLib.vmRunner config name` helper in lib.nix). Anything not
# listed is intentionally left out — the broker SpawnRunner
# pipeline generates runner argv in Rust
# (`packages/d2b-host/src/ch_argv.rs` + sibling
# `*_argv.rs` modules), so the Nix side only needs to surface
# the option values, not build runner derivations.
{ config, lib, pkgs, ... }:

let
  inherit (lib) mkOption types;
in
{
  options.microvm = {
    hypervisor = mkOption {
      type = types.enum [ "cloud-hypervisor" "crosvm" "qemu" "firecracker" "kvmtool" "stratovirt" ];
      default = "cloud-hypervisor";
      description = "Hypervisor binary that runs this VM.";
    };

    vcpu = mkOption {
      type = types.ints.positive;
      default = 1;
      description = "Number of vCPUs allocated to this VM.";
    };

    mem = mkOption {
      type = types.ints.positive;
      default = 512;
      description = "Memory in MiB allocated to this VM.";
    };

    hotplugMem = mkOption {
      type = types.ints.unsigned;
      default = 0;
      description = "Hotpluggable memory in MiB (0 = disabled).";
    };

    hotpluggedMem = mkOption {
      type = types.ints.unsigned;
      default = 0;
      description = "Currently-hotplugged memory in MiB (subset of hotplugMem).";
    };

    hugepageMem = mkOption {
      type = types.bool;
      default = false;
      description = "Whether to back guest memory with hugepages.";
    };

    balloon = mkOption {
      type = types.bool;
      default = false;
      description = "Whether the VM has a virtio-balloon device.";
    };

    initialBalloonMem = mkOption {
      type = types.ints.unsigned;
      default = 0;
      description = "Initial balloon size in MiB.";
    };

    deflateOnOOM = mkOption {
      type = types.bool;
      default = false;
      description = "Whether the balloon deflates on guest OOM.";
    };

    storeOnDisk = mkOption {
      type = types.bool;
      default = false;
      description = "Whether the guest's /nix/store is on a virtual disk image.";
    };

    storeDisk = mkOption {
      type = types.nullOr types.path;
      default = null;
      description = "Path to the store disk image (when storeOnDisk = true).";
    };

    writableStoreOverlay = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Optional writable overlay path on top of the read-only store.";
    };

    kernel = mkOption {
      type = types.attrsOf types.unspecified;
      default = pkgs.linuxPackages.kernel;
      defaultText = lib.literalExpression "pkgs.linuxPackages.kernel";
      description = "Kernel derivation for this VM. Must expose .dev and .out outputs.";
    };

    kernelParams = mkOption {
      type = types.listOf types.str;
      default = [ ];
      description = "Extra kernel command-line parameters.";
    };

    initrdPath = mkOption {
      type = types.path;
      default = config.system.build.initialRamdisk + "/initrd";
      defaultText = lib.literalExpression "config.system.build.initialRamdisk + \"/initrd\"";
      description = "Path to the initramfs image.";
    };

    vsock = {
      cid = mkOption {
        type = types.ints.positive;
        readOnly = true;
        description = "Host-owned per-VM AF_VSOCK CID.";
      };

      socket = mkOption {
        type = types.str;
        readOnly = true;
        description = "Host-owned Cloud Hypervisor base vsock Unix socket path.";
      };
    };

    interfaces = mkOption {
      type = types.listOf (types.submodule {
        options = {
          type = mkOption { type = types.enum [ "tap" "user" "macvtap" "bridge" ]; default = "tap"; };
          id = mkOption { type = types.str; };
          mac = mkOption { type = types.str; };
          bridge = mkOption { type = types.nullOr types.str; default = null; };
        };
      });
      default = [ ];
      description = "Per-VM network interfaces.";
    };

    shares = mkOption {
      type = types.listOf (types.submodule {
        options = {
          tag = mkOption { type = types.str; };
          source = mkOption { type = types.str; };
          mountPoint = mkOption { type = types.str; };
          proto = mkOption {
            type = types.enum [ "virtiofs" "9p" ];
            default = "virtiofs";
          };
          socket = mkOption { type = types.nullOr types.str; default = null; };
          readOnly = mkOption { type = types.bool; default = false; };
        };
      });
      default = [ ];
      description = "Per-VM virtiofs / 9p shares.";
    };

    devices = mkOption {
      type = types.listOf types.attrs;
      default = [ ];
      description = "Per-VM device passthrough entries (PCI, USB, etc.).";
    };

    volumes = mkOption {
      type = types.listOf types.attrs;
      default = [ ];
      description = "Per-VM extra volume images.";
    };

    cloud-hypervisor = {
      package = mkOption {
        type = types.package;
        default = pkgs.cloud-hypervisor;
        defaultText = lib.literalExpression "pkgs.cloud-hypervisor";
        description = "Cloud Hypervisor binary package.";
      };
      extraArgs = mkOption {
        type = types.listOf types.str;
        default = [ ];
        description = "Extra argv passed to cloud-hypervisor.";
      };
      platformOEMStrings = mkOption {
        type = types.listOf types.str;
        default = [ ];
        description = "OEM strings exposed via SMBIOS for systemd credentials.";
      };
    };

    virtiofsd = {
      package = mkOption {
        type = types.package;
        default = pkgs.virtiofsd;
        defaultText = lib.literalExpression "pkgs.virtiofsd";
        description = "virtiofsd binary package.";
      };
      threadPoolSize = mkOption {
        type = types.either types.ints.positive (types.enum [ "auto" ]);
        default = "auto";
        description = "Per-share virtiofsd thread pool size.";
      };
      group = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Per-share virtiofsd socket group ownership.";
      };
      inodeFileHandles = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "virtiofsd --inode-file-handles policy.";
      };
      extraArgs = mkOption {
        type = types.listOf types.str;
        default = [ ];
        description = "Extra argv passed to virtiofsd.";
      };
    };

    graphics = {
      enable = mkOption {
        type = types.bool;
        default = false;
        description = "Whether the VM has a graphics device.";
      };
      crosvmPackage = mkOption {
        type = types.package;
        default = pkgs.crosvm;
        defaultText = lib.literalExpression "pkgs.crosvm";
        description = "crosvm binary used for the GPU sidecar.";
      };
      socket = mkOption {
        type = types.str;
        default = "/run/d2b/vms/${config._module.args.name or "unknown"}/gpu.sock";
        description = "GPU device socket path.";
      };
    };

    extraArgsScript = mkOption {
      type = types.nullOr (types.either types.path types.str);
      default = null;
      description = ''
        Optional per-VM script that emits additional hypervisor argv
        on stdout at runner-start time (used by the audio guest module
        to inject `--generic-vhost-user` flags whose socket path is
        only known at boot-time). The broker reads this path via the
        bundle `runner-intent.extra_args_script` field; the Nix-side
        value is a path/string referencing the script derivation.

        Note: at v1.1.1 the broker's Rust argv generators do not yet
        honor this field (the generators emit static argv
        from typed inputs). The audio module's existing per-VM
        script-based injection is preserved on the bundle side for
        backward compatibility with the daemon's pre-spawn argv
        envelope; the broker spawn path reads the prebuilt argv from
        the bundle which already contains the script invocation.
      '';
    };

    # declaredRunner is NOT emitted by the d2b-owned evaluator.
    # The broker spawns the hypervisor directly via the
    # Rust argv generators in `packages/d2b-host/src/*_argv.rs`;
    # no Nix-side runner derivation is needed in v1.1+.
    declaredRunner = mkOption {
      type = types.nullOr types.package;
      default = null;
      internal = true;
      description = ''
        Always null in v1.1+ (d2b owns the substrate; the broker
        Rust argv generators replace microvm.nix's runner derivation).
        Preserved as a typed `null` for backward-compat with consumers
        that touch the path.
      '';
    };
  };
}
