# tests/unit/smoke/smoke-eval-home-manager.nix — v0.1.0 H4 regression test.
#
# Mirrors tests/unit/smoke/smoke-eval.nix but requests the Home Manager
# workload capability and supplies guest Home Manager users. Exercises
# the codepath that imports
# `inputs.home-manager.nixosModules.home-manager` via
# `nixos-modules/components/home-manager.nix`. Before v0.1.0 H4
# the `home-manager` input wasn't declared on the root flake, so
# any consumer that enabled the Home Manager workload path hit
# `attribute 'home-manager' missing` at eval time. This test would
# have caught that regression.
#
# Wired into tests/static.sh as a Layer-1 gate.
{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
}:

let
  inherit (pkgs) lib;

  flake = builtins.getFlake "git+file://${toString ./../../..}";
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;

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
            launcher.capabilities = [ "home-manager" ];
            config = {
              networking.hostName = lib.mkDefault "corp-vm";
              users.users.alice = {
                isNormalUser = true;
                uid = 1000;
              };
              d2b.homeManager.users.alice = { lib, ... }: {
                home.username = "alice";
                home.homeDirectory = "/home/alice";
                home.stateVersion = "25.11";
              };
            };
          };
        };
      })
    ];
  };
in
  nixos.config.system.build.toplevel
