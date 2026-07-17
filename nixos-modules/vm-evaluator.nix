{ inputs }:

{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  inherit (lib) mkOption types;

  workloadGuestOptions =
    { config, d2bRealmId, d2bWorkloadId, d2bRoleIds, ... }:
    let
      roleSocket = roleKind: file:
        "/run/d2b/r/${d2bRealmId}/w/${d2bWorkloadId}/roles/${
          d2bRoleIds.${roleKind}
        }/${file}";
    in
    {
      options.microvm = {
        hypervisor = mkOption {
          type = types.enum [ "cloud-hypervisor" "qemu" ];
          default = "cloud-hypervisor";
        };
        vcpu = mkOption {
          type = types.ints.positive;
          default = 1;
        };
        mem = mkOption {
          type = types.ints.positive;
          default = 512;
        };
        hotplugMem = mkOption {
          type = types.ints.unsigned;
          default = 0;
        };
        hotpluggedMem = mkOption {
          type = types.ints.unsigned;
          default = 0;
        };
        hugepageMem = mkOption {
          type = types.bool;
          default = false;
        };
        balloon = mkOption {
          type = types.bool;
          default = false;
        };
        initialBalloonMem = mkOption {
          type = types.ints.unsigned;
          default = 0;
        };
        deflateOnOOM = mkOption {
          type = types.bool;
          default = false;
        };
        storeOnDisk = mkOption {
          type = types.bool;
          default = false;
        };
        storeDisk = mkOption {
          type = types.nullOr types.path;
          default = null;
        };
        writableStoreOverlay = mkOption {
          type = types.nullOr types.str;
          default = null;
        };
        kernel = mkOption {
          type = types.attrsOf types.unspecified;
          default = pkgs.linuxPackages.kernel;
        };
        kernelParams = mkOption {
          type = types.listOf types.str;
          default = [ ];
        };
        initrdPath = mkOption {
          type = types.path;
          default = config.system.build.initialRamdisk + "/initrd";
        };
        vsock = {
          cid = mkOption {
            type = types.ints.positive;
            readOnly = true;
          };
          socket = mkOption {
            type = types.str;
            readOnly = true;
          };
        };
        interfaces = mkOption {
          type = types.listOf types.attrs;
          default = [ ];
        };
        shares = mkOption {
          type = types.listOf types.attrs;
          default = [ ];
        };
        devices = mkOption {
          type = types.listOf types.attrs;
          default = [ ];
        };
        volumes = mkOption {
          type = types.listOf types.attrs;
          default = [ ];
        };
        cloud-hypervisor = {
          package = mkOption {
            type = types.package;
            default = pkgs.cloud-hypervisor;
          };
          extraArgs = mkOption {
            type = types.listOf types.str;
            default = [ ];
          };
          platformOEMStrings = mkOption {
            type = types.listOf types.str;
            default = [ ];
          };
        };
        virtiofsd = {
          package = mkOption {
            type = types.package;
            default = pkgs.virtiofsd;
          };
          threadPoolSize = mkOption {
            type = types.either types.ints.positive (types.enum [ "auto" ]);
            default = "auto";
          };
          group = mkOption {
            type = types.nullOr types.str;
            default = null;
          };
          extraArgs = mkOption {
            type = types.listOf types.str;
            default = [ ];
          };
        };
        graphics = {
          enable = mkOption {
            type = types.bool;
            default = false;
          };
          crosvmPackage = mkOption {
            type = types.package;
            default = pkgs.crosvm;
          };
          renderNodeOnly = mkOption {
            type = types.bool;
            default = false;
          };
          socket = mkOption {
            type = types.str;
            default =
              if config.microvm.graphics.renderNodeOnly
              then roleSocket "gpu-render-node" "gpu.sock"
              else roleSocket "gpu" "gpu.sock";
          };
        };
      };
    };

  evalWorkload = workload: composedModules:
    let
      roles = cfg._index.roles.byWorkloadId.${workload.workloadId} or [ ];
      roleIds = lib.listToAttrs (map
        (role: {
          name = role.roleKind;
          value = role.roleId;
        })
        roles);
      evaluated = import (pkgs.path + "/nixos/lib/eval-config.nix") {
        modules = [
          workloadGuestOptions
          ./vm-guest-base.nix
          ./guest-control.nix
          {
            nixpkgs.config = config.nixpkgs.config;
            nixpkgs.overlays = config.nixpkgs.overlays;
          }
          {
            _module.args.name = workload.workloadId;
          }
        ] ++ composedModules;
        specialArgs =
          {
            inherit inputs;
            d2bInputs = inputs;
            d2bRealmId = workload.realmId;
            d2bWorkloadId = workload.workloadId;
            d2bRoleIds = roleIds;
          }
          // cfg.site.extraSpecialArgs;
        inherit (pkgs.stdenv.hostPlatform) system;
      };
    in
    {
      inherit (evaluated) config options;
      inherit roleIds;
    };
in
{
  _composeWorkload = evalWorkload;
  config = { };
}
