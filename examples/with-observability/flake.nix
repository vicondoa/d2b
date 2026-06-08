{
  description = "nixling example: one workload VM with the auto-declared observability stack";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    microvm = {
      url = "github:microvm-nix/microvm.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    nixling = {
      url = "path:../..";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.microvm.follows = "microvm";
      inputs.home-manager.follows = "home-manager";
    };
  };

  outputs = { nixpkgs, nixling, ... }: {
    nixosConfigurations.demo = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        nixling.nixosModules.default
        ({ lib, ... }: {
          boot.loader.grub.enable = false;
          boot.loader.systemd-boot.enable = true;
          boot.loader.efi.canTouchEfiVariables = false;
          boot.initrd.includeDefaultModules = false;
          fileSystems."/" = {
            device = "tmpfs";
            fsType = "tmpfs";
          };
          environment.etc."machine-id".text =
            "00000000000000000000000000000000";

          networking.hostName = "demo";
          system.stateVersion = "25.11";

          users.users.alice = {
            isNormalUser = true;
            uid = 1000;
          };

          nixling.site = {
            waylandUser = null;
            launcherUsers = [ ];
            yubikey.enable = false;
          };

          nixling.observability.enable = true;

          nixling.envs.work = {
            lanSubnet = "10.20.0.0/24";
            uplinkSubnet = "192.0.2.0/30";
          };

          nixling.vms.work-app = {
            enable = true;
            env = "work";
            index = 10;
            ssh.user = "alice";
            observability.enable = true;

            config = {
              networking.hostName = lib.mkDefault "work-app";
              users.users.alice = {
                isNormalUser = true;
                uid = 1000;
              };
            };
          };
        })
      ];
    };
  };
}
