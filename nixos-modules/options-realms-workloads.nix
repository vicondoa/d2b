# d2b.realms.<realm>.workloads.<workload> — realm-native workload
# declaration schema.
#
# This file is imported as a fragment inside the d2b.realms.<realm>
# submodule (see options-realms.nix).  It declares the
# `workloads` attrsOf-submodule option and all per-workload sub-options.
#
# Supported kinds
# ---------------
# "local-vm"   — NixOS guest running on Cloud Hypervisor, managed by
#                d2bd.  Mirrors d2b.vms.<vm> with runtime.kind = "nixos".
# "qemu-media" — External-media QEMU runner.  Mirrors
#                d2b.vms.<vm> with runtime.kind = "qemu-media".
# "provider-placeholder"
#              — Placeholder for a provider-managed workload whose
#                runtime is not instantiated locally.  Schema foundation
#                only; no daemon process is started.
#
# State path policy
# -----------------
# Each workload maps its primary state directory 1:1 to the legacy
# /var/lib/d2b/vms/<workload-id> path by default.  No activation-time
# state migration occurs; if the workload-id matches an existing
# d2b.vms name the paths are identical and existing data is preserved.
{ lib, config, name, ... }:

let
  realmId = config.id;

  # Regex types re-used from options-vms.nix for consistency.
  qemuMediaRefType = lib.types.strMatching "^[a-z][a-z0-9-]{0,62}$";
  qemuMediaByIdNameType = lib.types.strMatching "^[A-Za-z0-9._:+-]{1,255}$";

  qemuMediaSourceType = lib.types.submodule {
    freeformType = null;
    options = {
      ref = lib.mkOption {
        type = lib.types.nullOr qemuMediaRefType;
        default = null;
        example = "installer-usb";
        description = "Opaque media reference for physical-usb sources.";
      };

      path = lib.mkOption {
        type = lib.types.nullOr (lib.types.strMatching "^/.*$");
        default = null;
        example = "/var/lib/d2b/images/installer.img";
        description = "Absolute host path for an image-file source.";
      };

      usbSelector = lib.mkOption {
        type = lib.types.nullOr (lib.types.submodule {
          freeformType = null;
          options = {
            byIdName = lib.mkOption {
              type = qemuMediaByIdNameType;
              example = "usb-Example_Flash_Disk_123456-0:0";
              description = "Exact /dev/disk/by-id basename used to resolve the physical USB source at runtime.";
            };
          };
        });
        default = null;
        description = "Optional stable selector for physical-usb sources.";
      };

      kind = lib.mkOption {
        type = lib.types.enum [ "physical-usb" "image-file" ];
        default = "physical-usb";
        description = "Source class for the opaque media reference.";
      };

      format = lib.mkOption {
        type = lib.types.enum [ "raw" "qcow2" "iso" ];
        default = "raw";
        description = "QEMU block format hint. The unsafe QEMU format-probing default is never used.";
      };

      readOnly = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether the media source is attached read-only.";
      };
    };
  };

  qemuMediaSlotType = lib.types.submodule ({ name, ... }: {
    freeformType = null;
    options = {
      source = lib.mkOption {
        type = lib.types.nullOr qemuMediaSourceType;
        default = null;
        description = "Optional media source currently inserted in removable slot ${name}.";
      };

      readOnly = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Default read-only policy for media inserted into this slot.";
      };
    };
  });

  # Desktop launcher action type.
  launcherActionType = lib.types.submodule {
    freeformType = null;
    options = {
      id = lib.mkOption {
        type = lib.types.str;
        example = "open-terminal";
        description = "Machine-stable identifier for this desktop action.";
      };

      label = lib.mkOption {
        type = lib.types.str;
        example = "Open Terminal";
        description = "Human-readable label for this desktop launcher action.";
      };

      command = lib.mkOption {
        type = lib.types.str;
        example = "d2b vm exec workstation -- bash -l";
        description = "Shell command invoked when the action is triggered.";
      };
    };
  };

  workloadSubmodule = lib.types.submodule ({ name, config, ... }:
    let
      workloadId = config.id;
      defaultStateDir = "/var/lib/d2b/vms/${workloadId}";
      defaultRunDir = "/run/d2b/vms/${workloadId}";
    in
    {
      freeformType = null;
      options = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = "Whether this workload is active in the realm.";
        };

        id = lib.mkOption {
          type = lib.types.strMatching "^[a-z][a-z0-9-]*$";
          default = name;
          description = ''
            Stable workload identifier.  Defaults to the attribute name.
            Used to derive state paths and unit names.
          '';
        };

        kind = lib.mkOption {
          type = lib.types.enum [ "local-vm" "qemu-media" "provider-placeholder" ];
          default = "local-vm";
          description = ''
            Runtime family for this workload.

            `local-vm`            — NixOS guest on Cloud Hypervisor; d2bd
                                    supervises the lifecycle DAG.
            `qemu-media`          — External-media QEMU runner for live/
                                    installer media or opaque OS images.
            `provider-placeholder`— Schema-only placeholder for a
                                    provider-managed workload; no local
                                    runtime process is started.
          '';
        };

        legacyVmName = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "laptop";
          description = ''
            Optional reference to an existing `d2b.vms.<name>` entry.
            When set, the workload's state path defaults to the legacy
            VM's path without any activation-time state migration.  Use
            this during the env→realm network transition to keep state
            at the existing location while adding realm metadata.
          '';
        };

        stateDir = lib.mkOption {
          type = lib.types.strMatching "^/.*$";
          default = defaultStateDir;
          defaultText = lib.literalExpression "\"/var/lib/d2b/vms/<workload-id>\"";
          description = ''
            Primary state directory for this workload.  The default maps
            1:1 to the legacy `d2b.vms.<vm>` path so existing on-disk
            state (TPM, store-view, audio state, guest-control token, …)
            is preserved without any activation-time migration.
          '';
        };

        runDir = lib.mkOption {
          type = lib.types.strMatching "^/.*$";
          default = defaultRunDir;
          defaultText = lib.literalExpression "\"/run/d2b/vms/<workload-id>\"";
          description = "Runtime directory for this workload.";
        };

        # ----------------------------------------------------------------
        # local-vm options
        # ----------------------------------------------------------------

        localVm = {
          memoryMiB = lib.mkOption {
            type = lib.types.nullOr lib.types.ints.positive;
            default = null;
            example = 4096;
            description = ''
              Guest RAM in MiB for `kind = "local-vm"`.  When null the
              NixOS guest configuration declares its own `microvm.mem`.
            '';
          };

          vcpus = lib.mkOption {
            type = lib.types.nullOr lib.types.ints.positive;
            default = null;
            example = 4;
            description = ''
              Virtual CPU count for `kind = "local-vm"`.  When null
              the guest configuration declares `microvm.vcpu`.
            '';
          };

          networkIndex = lib.mkOption {
            type = lib.types.nullOr (lib.types.ints.between 10 250);
            default = null;
            example = 20;
            description = ''
              Workload IP index within the realm's LAN subnet.
              The workload's IP = <lan-subnet-prefix>.<networkIndex>.
              Must be unique within the realm when set.
            '';
          };

          config = lib.mkOption {
            type = lib.types.deferredModule;
            default = { };
            example = lib.literalExpression "{ imports = [ ../../vms/laptop.nix ]; }";
            description = ''
              NixOS module merged into the local-vm guest's configuration.
              Equivalent to `d2b.vms.<vm>.config`.
            '';
          };

          autostart = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = "Start this workload at host boot.";
          };

          ssh = {
            user = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "alice";
              description = "Username for d2b-driven SSH / guest-exec access to this workload.";
            };
          };

          graphics = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Enable virtio-gpu + Wayland cross-domain for this workload.";
            };
          };

          tpm = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Enable swtpm-backed TPM 2.0 device for this workload.";
            };
          };
        };

        # ----------------------------------------------------------------
        # qemu-media options
        # ----------------------------------------------------------------

        qemuMedia = {
          source = lib.mkOption {
            type = lib.types.nullOr qemuMediaSourceType;
            default = null;
            description = "Primary OS/boot media source for `kind = \"qemu-media\"`.";
          };

          removableSlots = lib.mkOption {
            type = lib.types.attrsOf qemuMediaSlotType;
            default = { };
            description = "Named removable-media slots for the QEMU media runner.";
          };

          bootDrive = {
            slot = lib.mkOption {
              type = lib.types.strMatching "^(boot|[a-z][a-z0-9-]{0,62})$";
              default = "boot";
              description = ''
                Slot selected as the intended boot drive.  `boot` selects
                `qemuMedia.source`; any other value names a
                `qemuMedia.removableSlots.<slot>` entry.
              '';
            };
          };

          resources = {
            memoryMiB = lib.mkOption {
              type = lib.types.ints.positive;
              default = 4096;
              description = "Guest RAM for the qemu-media workload, in MiB.";
            };

            vcpu = lib.mkOption {
              type = lib.types.ints.positive;
              default = 2;
              description = "Virtual CPU count for the qemu-media workload.";
            };
          };

          security = {
            lockMemory = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Lock guest RAM with QEMU mem-lock=on.";
            };

            excludeMemoryFromCoreDump = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = "Exclude qemu-media guest RAM from QEMU memory dumps.";
            };

            disableMemoryMerge = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = "Disable Kernel Samepage Merging for qemu-media guest RAM.";
            };
          };
        };

        # ----------------------------------------------------------------
        # Desktop launcher metadata
        # ----------------------------------------------------------------

        launcher = {
          enable = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = ''
              Whether to include this workload in generated desktop-launcher
              metadata (`.desktop` file entries, Waybar display lists,
              d2b-wlterm integration, etc.).
            '';
          };

          label = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "Work Laptop";
            description = ''
              Human-readable display label for this workload in desktop
              launchers.  When null, the workload id is used.
            '';
          };

          icon = {
            id = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "computer-laptop";
              description = ''
                XDG icon theme id for this workload's launcher icon.  Used
                when generating `.desktop` files and desktop-metadata JSON.
              '';
            };

            name = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "laptop";
              description = ''
                Short symbolic icon name; used as a fallback when `icon.id`
                does not resolve in the running icon theme.
              '';
            };
          };

          app = {
            command = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "d2b vm exec workstation -- bash -l";
              description = ''
                Primary application/launch command for this workload.
                Emitted as the `Exec=` field in generated `.desktop`
                metadata.  When null, d2b derives a default based on the
                workload kind and available guest-control capabilities.
              '';
            };

            targetRealm = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              defaultText = lib.literalExpression "\"<workload-id>.<realm-id>.d2b\"";
              description = ''
                Canonical realm-qualified target address for desktop tooling
                that needs to route to this workload.  When null, d2b derives
                `<workload-id>.<realm-id>.d2b`.
              '';
            };
          };

          actions = lib.mkOption {
            type = lib.types.listOf launcherActionType;
            default = [ ];
            description = ''
              Additional named actions exposed in the desktop launcher (e.g.
              "Open Terminal", "Restart VM").  Each action is emitted as a
              `[Desktop Action <id>]` section in generated `.desktop` metadata.
            '';
          };

          capabilities = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            example = [ "guest-exec" "persistent-shell" "graphics" ];
            description = ''
              Optional capability identifiers required for this workload's
              launcher to function correctly.  Desktop tooling may use these
              to pre-flight capability availability and surface actionable
              messages when a required capability is absent.

              Well-known values: `guest-exec`, `persistent-shell`,
              `graphics`, `audio`, `tpm`, `usb-security-key`.
            '';
          };
        };
      };
    });
in
{
  options.workloads = lib.mkOption {
    type = lib.types.attrsOf workloadSubmodule;
    default = { };
    description = ''
      Realm-owned workload declarations.  Each workload maps to a runtime
      entity supervised by d2bd: a local NixOS VM (`kind = "local-vm"`),
      an external-media QEMU runner (`kind = "qemu-media"`), or a
      schema-only provider placeholder (`kind = "provider-placeholder"`).

      Workload state directories default to the legacy
      `/var/lib/d2b/vms/<workload-id>` path, preserving on-disk state
      for any workload whose id matches an existing `d2b.vms.<vm>` name.
    '';
    example = lib.literalExpression ''
      {
        laptop = {
          kind = "local-vm";
          legacyVmName = "laptop";
          localVm.ssh.user = "alice";
          localVm.graphics.enable = true;
          launcher = {
            enable = true;
            label = "Work Laptop";
            icon.id = "computer-laptop";
            capabilities = [ "guest-exec" "graphics" ];
          };
        };
        installer = {
          kind = "qemu-media";
          qemuMedia.source = {
            kind = "physical-usb";
            ref = "installer-usb";
          };
          launcher.enable = true;
          launcher.label = "Live Installer";
        };
      }
    '';
  };
}
