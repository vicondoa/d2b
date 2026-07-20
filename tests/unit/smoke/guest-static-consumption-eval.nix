{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
, flake ? builtins.getFlake ("git+file://" + toString ./../../..)
}:

let
  inherit (pkgs) lib;
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  expected = [
    flake.packages.${system}.d2b-guestd-static
    flake.packages.${system}.d2b-exec-runner-static
  ];
  nixos = nixosSystem {
    inherit system;
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        boot.loader.grub.enable = false;
        boot.loader.systemd-boot.enable = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = {
          device = "tmpfs";
          fsType = "tmpfs";
        };
        environment.etc."machine-id".text = "00000000000000000000000000000000";
        system.stateVersion = "25.11";

        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };

        d2b.site = {
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
          extraSpecialArgs.inputs = { };
        };

        d2b.acceptDestructiveV2Cutover = true;
        d2b.realms.work = {
          path = "work";
          placement = "host-local";
          broker = {
            enable = true;
            hostMutation = true;
          };
          network = {
            mode = "declared";
            lanSubnet = "10.20.0.0/24";
            uplinkSubnet = "192.0.2.0/30";
          };
          providers.runtime = {
            type = "runtime";
            implementationId = "cloud-hypervisor";
          };
          workloads.corp-vm = {
            providerRefs.runtime = "runtime";
            config = {
              networking.hostName = lib.mkDefault "corp-vm";
              users.users.alice = {
                isNormalUser = true;
                uid = 1000;
              };
            };
          };
        };
      })
    ];
  };
  workload = lib.findFirst
    (row: row.workloadName == "corp-vm")
    (throw "corp-vm workload missing from normalized index")
    nixos.config.d2b._index.workloads.enabledList;
  guestSystemPackages =
    nixos.config.d2b._computedWorkloads.${workload.workloadId}
      .config.environment.systemPackages;
in
assert lib.all (pkg: builtins.elem pkg guestSystemPackages) expected;
builtins.toJSON (map toString expected)
