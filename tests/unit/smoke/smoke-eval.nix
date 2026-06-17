# tests/unit/smoke/smoke-eval.nix — Layer-1 smoke evaluation for nixling.
#
# Minimal consumer-style `nixosSystem` that imports
# `nixling.nixosModules.default` and exercises the parts of the eval
# graph that are touched by every consumer:
#
#   - nixling.site.* defaults flow through.
#   - At least one nixling.envs.<env> materialises (network.nix
#     allMeta non-empty → route-preflight unit + assertion block).
#   - At least one nixling.vms.<name> with an env reference round-trips
#     through host.nix's microvm.vms translation.
#   - All component toggles default off (graphics/audio/tpm/usbip),
#     so the heavyweight component imports (graphics.nix, etc.) stay
#     out of this smoke path. The test is fast.
#
# Returns `system.build.toplevel` — building it would pull the whole
# closure, but `nix eval --raw` on this attribute only forces the
# derivation path string (a fully evaluated nixos config). That's
# enough to catch regressions in:
#   - `nixosModules.default`'s partial-application wiring of `inputs`.
#   - Assertion-block fire-time errors (graphics-without-waylandUser,
#     CIDR shape, key validation, CIDR overlap, …).
#   - Option-schema typos in the public surface.
#
# Run via:
#   nix eval --raw -f tests/unit/smoke/smoke-eval.nix
#   # or, from the flake:
#   nix build .#checks.x86_64-linux.smoke
#
# Wired into tests/static.sh as a Layer-1 gate.
{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
}:

let
  inherit (pkgs) lib;

  # Import the flake-as-source via getFlake. Path relative to this
  # file so the test works regardless of caller cwd.
  flake = builtins.getFlake "git+file://${toString ./../../..}";

  # `nixosSystem` lives on the nixpkgs flake's `lib`, not on
  # `pkgs.lib`. Pull it from the flake graph; this keeps the smoke
  # test independent of the host's nix-channel state.
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;

  nixos = nixosSystem {
    inherit system;
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        # Minimal NixOS baseline so the eval graph can resolve. None
        # of these knobs is exercised by nixling itself; they exist to
        # make `system.build.toplevel` reachable without a real disk
        # or bootloader.
        boot.loader.grub.enable = false;
        boot.loader.systemd-boot.enable = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = {
          device = "tmpfs";
          fsType = "tmpfs";
        };
        # microvm.nix's host module pulls in /etc/machine-id assertions;
        # provide a placeholder.
        environment.etc."machine-id".text = "00000000000000000000000000000000";
        system.stateVersion = "25.11";

        # Single consumer-side user that satisfies waylandUser +
        # launcherUsers + ssh.user references. Stick to the
        # documentation placeholder set (`alice` / `contoso.com`).
        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };

        # Site-level: minimum surface to satisfy graphics/audio
        # assertions if those toggles ever flip in this smoke
        # config. Both stay off by default below.
        nixling.site = {
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          # Toggle off the host-side Yubico bits — smoke config
          # has no use for them and exercising the .enable=false
          # path is the more interesting one to keep regression-
          # free.
          yubikey.enable = false;
        };

        # One env — exercises network.nix materialisation, the
        # route preflight unit, and the CIDR validation chain.
        nixling.envs.work = {
          lanSubnet    = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };

        # One workload VM — exercises host.nix's microvm.vms
        # translation, the framework-managed SSH key activation,
        # and the cli.nix manifest builder.
        nixling.vms.corp-vm = {
          enable = true;
          env = "work";
          index = 10;
          ssh.user = "alice";
          # ssh.keyPath intentionally LEFT NULL so the C2 default
          # (derived from cfg.site.keysDir) is exercised in the
          # manifest builder.
          config = {
            networking.hostName = lib.mkDefault "corp-vm";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };
      })
    ];
  };
in
  nixos.config.system.build.toplevel
