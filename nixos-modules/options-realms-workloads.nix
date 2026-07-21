# d2b.realms.<realm>.workloads.<workload> — provider-bound workloads.
{ config, lib, ... }:

let
  labelType = lib.types.strMatching "^[a-z][a-z0-9-]{0,127}$";

  launcherItemType = lib.types.submodule ({ name, ... }: {
    freeformType = null;
    options = {
      type = lib.mkOption {
        type = lib.types.enum [ "exec" "shell" ];
        default = "exec";
        description = "Provider-neutral launcher item operation.";
      };

      name = lib.mkOption {
        type = lib.types.str;
        default = name;
        description = "Human-readable launcher item name.";
      };

      icon = {
        id = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Optional XDG icon theme id.";
        };

        name = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Optional symbolic icon name.";
        };
      };

      argv = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = "Private configured argv for an exec launcher item.";
      };

      graphical = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Whether this item requires the mediated display provider.";
      };
    };
  });

  workloadType = lib.types.submodule ({ name, ... }: {
    freeformType = null;
    options = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether this workload is active.";
      };

      id = lib.mkOption {
        type = labelType;
        default = name;
        description = "Stable workload identifier; defaults to the attribute name.";
      };

      name = lib.mkOption {
        type = lib.types.str;
        default = name;
        description = "Human-readable workload name.";
      };

      providerRefs = lib.mkOption {
        type = lib.types.attrsOf labelType;
        default = { };
        example = {
          runtime = "vm";
          device = "devices";
          display = "wayland";
        };
        description = ''
          Explicit bindings from a closed provider authority name to an enabled
          provider instance in this realm. Runtime-backed workloads must bind
          `runtime`; feature options bind their matching device, display,
          audio, network, storage, or transport authority.
        '';
      };

      config = lib.mkOption {
        type = lib.types.deferredModule;
        default = { };
        description = "Declarative workload module consumed by the selected runtime provider.";
      };

      autostart = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Whether the owning realm controller starts this workload automatically.";
      };

      tpm.enable = lib.mkEnableOption "a stateful TPM 2.0 device for this workload";

      graphics = {
        enable = lib.mkEnableOption "mediated GPU graphics for this workload";

        videoSidecar = lib.mkEnableOption "the mediated H.264 video decode sidecar";

        videoNvidiaDecode = lib.mkEnableOption "NVIDIA device mediation for the video sidecar";

        renderNodeOnly = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Expose only the mediated render-node role.";
        };
      };

      audio = {
        enable = lib.mkEnableOption "vhost-user audio mediation for this workload";

        allowMicByDefault = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Initial microphone policy; runtime grants remain provider-mediated.";
        };

        allowSpeakerByDefault = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Initial speaker policy; runtime grants remain provider-mediated.";
        };
      };

      usbip.enable = lib.mkEnableOption "exclusive USBIP device mediation for this workload";

      securityKey.enable =
        lib.mkEnableOption "mediated FIDO security-key ceremonies for this workload";

      display.wayland = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Attach the workload to its explicitly bound Wayland display provider.";
      };

      guestControl.vsockRelay = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Render the authenticated guest-control vsock relay role.";
      };

      shell = {
        enable = lib.mkEnableOption "persistent-shell capability for this workload";

        defaultName = lib.mkOption {
          type = lib.types.strMatching "^[A-Za-z0-9_][A-Za-z0-9._-]{0,63}$";
          default = "default";
          description = "Default persistent shell session name.";
        };

        maxSessions = lib.mkOption {
          type = lib.types.ints.between 1 64;
          default = 8;
          description = "Maximum persistent shell sessions for this workload.";
        };
      };

      _frameworkReservedName = lib.mkOption {
        type = lib.types.bool;
        default = false;
        internal = true;
        visible = false;
        description = ''
          Internal attestation, set only by a framework module (this file's
          own auto-declared `network` workload, or components/observability
          via its auto-declared `sys-*` stack workload), that this workload's
          otherwise-reserved name is legitimately framework-owned. Operator
          configuration must never set this option: it exists solely so the
          reserved-name assertions in options-realms.nix can distinguish a
          framework auto-declaration from an operator name collision.
        '';
      };

      launcher = {
        enable = lib.mkEnableOption "desktop launcher metadata for this workload";

        label = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Optional desktop display label.";
        };

        icon = {
          id = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            description = "Optional XDG icon theme id.";
          };

          name = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            description = "Optional symbolic icon name.";
          };
        };

        defaultItem = lib.mkOption {
          type = lib.types.nullOr labelType;
          default = null;
          description = "Launcher item selected when no item is specified.";
        };

        items = lib.mkOption {
          type = lib.types.attrsOf launcherItemType;
          default = { };
          description = "Provider-neutral launcher items keyed by stable item id.";
        };

        capabilities = lib.mkOption {
          type = lib.types.listOf labelType;
          default = [ ];
          description = "Capabilities required by this workload's launcher.";
        };
      };
    };
  });
in
{
  options.workloads = lib.mkOption {
    type = lib.types.attrsOf workloadType;
    default = { };
    description = ''
      Realm-owned workloads with closed feature configuration and explicit
      typed provider bindings. Legacy VM kinds, aliases, state mappings, and
      provider placeholders are not part of this schema.
    '';
  };

  # A realm that owns its network declaration (network.mode == "declared")
  # auto-declares its own net VM as an ordinary provider-bound workload
  # named "network", bound to the reserved cloud-hypervisor runtime
  # provider "network-vm-runtime". This is a self-referential sibling
  # default (config.network.mode -> config.workloads/providers) resolved
  # entirely within this realm instance's own submodule fixed point; it
  # never reads the fully-merged `d2b.realms` and so cannot recurse. The
  # guest module import (./net.nix) is deferred here; the realm-derived
  # network guest arguments (`realmNetwork`) are threaded in later from
  # host.nix, which is free to read the already-computed realm-network
  # rows without creating a cycle.
  config = lib.mkIf (config.network.mode == "declared") {
    providers.network-vm-runtime = {
      type = "runtime";
      implementationId = "cloud-hypervisor";
    };

    workloads.network = {
      providerRefs.runtime = "network-vm-runtime";
      autostart = true;
      config.imports = [ ./net.nix ];
      _frameworkReservedName = true;
    };
  };
}
