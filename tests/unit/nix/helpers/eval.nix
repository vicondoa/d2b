{ lib
, pkgs
, system
, nixpkgs
, d2bModule
, d2bLib
, flakeRoot
, moduleFixtures ? [ ]
}:

let
  support = import ../../../../nix/test-support/eval-surface.nix { inherit lib; };
  nixpkgsPath = nixpkgs.outPath or nixpkgs;

  sinkNamespaces = [
    "users"
    "system"
    "services"
    "environment"
    "boot"
    "networking"
    "security"
    "documentation"
    "time"
    "nix"
    "i18n"
    "hardware"
    "fileSystems"
    "swapDevices"
    "powerManagement"
    "programs"
    "console"
    "fonts"
    "sound"
    "virtualisation"
    "specialisation"
    "zramSwap"
    "xdg"
    "qt"
  ];

  mkSink = name: { lib, ... }: {
    options.${name} = lib.mkOption {
      type = lib.types.anything;
      default = { };
    };
  };

  systemdSink = { lib, ... }: {
    options.systemd = lib.mkOption {
      default = { };
      type = lib.types.submodule {
        freeformType = lib.types.attrsOf lib.types.anything;
        options.services = lib.mkOption {
          type = lib.types.attrsOf lib.types.anything;
          default = { };
        };
        options.user = lib.mkOption {
          type = lib.types.anything;
          default = { };
        };
        options.tmpfiles = lib.mkOption {
          default = { };
          type = lib.types.submodule {
            freeformType = lib.types.attrsOf lib.types.anything;
            options.rules = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
            };
          };
        };
      };
    };
  };

  nixpkgsSink = { lib, ... }: {
    options.nixpkgs = lib.mkOption {
      type = lib.types.anything;
      default = {
        config = { };
        overlays = [ ];
      };
    };
  };

  sinkModules = (builtins.map mkSink sinkNamespaces) ++ [
    systemdSink
    nixpkgsSink
  ];

  baseModuleFor = evalSystem: { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = {
      device = "tmpfs";
      fsType = "tmpfs";
    };
    environment.etc."machine-id".text =
      "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = {
      isNormalUser = true;
      uid = 1000;
    };
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
      usePrebuiltHostTools = lib.mkDefault (evalSystem == "x86_64-linux");
    };
    d2b.zones.local-root = { };
  };

  pkgsFor = evalSystem:
    if evalSystem == system then
      pkgs
    else
      import nixpkgsPath {
        system = evalSystem;
        config.allowUnsupportedSystem = true;
      };

  defaultSystem = system;
  mkEvalFor =
    { system ? defaultSystem
    , modules
    }:
    support.evalModules {
      modules = [
        "${nixpkgsPath}/nixos/modules/misc/assertions.nix"
        d2bModule
        (baseModuleFor system)
      ] ++ modules ++ moduleFixtures ++ sinkModules;
      specialArgs = {
        lib = (pkgsFor system).lib;
        pkgs = pkgsFor system;
        modulesPath = "${nixpkgsPath}/nixos/modules";
      };
    };

  mkEval = modules: mkEvalFor { inherit modules; };
  mkModuleEval = modules:
    support.evalModules {
      modules = [
        {
          _module.check = false;
        }
        d2bModule
      ] ++ modules;
      specialArgs = {
        lib = pkgs.lib;
        inherit pkgs;
        modulesPath = "${nixpkgsPath}/nixos/modules";
      };
    };

  # Guest-only modules such as net.nix take envMeta through specialArgs and
  # must not be imported as host modules. Declare the networkd attrset so
  # mkForce/mkDefault merge at the 10-eth-dhcp key.
  mkGuestEval =
    { modules
    , specialArgs ? { }
    }:
    support.evalModules {
      modules = [
        { _module.check = false; }
        {
          options.systemd.network.networks = lib.mkOption {
            type = lib.types.attrsOf lib.types.anything;
            default = { };
          };
        }
      ] ++ modules;
      specialArgs = {
        inherit lib pkgs;
        modulesPath = "${nixpkgsPath}/nixos/modules";
      } // specialArgs;
    };
in
{
  baseModule = baseModuleFor system;
  inherit lib pkgs system flakeRoot d2bLib d2bModule mkEval mkEvalFor mkModuleEval mkGuestEval;
  nixpkgsFlake = nixpkgs;
  inherit pkgsFor;
}
