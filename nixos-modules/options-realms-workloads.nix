# d2b.realms.<realm>.workloads.<workload> — provider-bound workloads.
{ lib, ... }:

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

      provider = lib.mkOption {
        type = labelType;
        description = "Enabled runtime provider instance in the owning realm.";
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
      Realm-owned workloads. A workload selects an enabled runtime provider by
      instance id; legacy VM kinds, provider-placeholder kinds, and legacy VM
      state mappings are not part of this schema.
    '';
  };
}
