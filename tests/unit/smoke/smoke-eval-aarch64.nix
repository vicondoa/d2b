# tests/unit/smoke/smoke-eval-aarch64.nix — multi-arch eval gate.
#
# Sibling of tests/unit/smoke/smoke-eval.nix that pins the eval-target system to
# `aarch64-linux`. Exercises the case the published refactor plan
# explicitly requires us to preserve: a headless workload VM (no
# `graphics.enable`, no `audio.enable`) MUST evaluate clean on
# aarch64-linux even though the cloud-hypervisor + crosvm +
# vhost-device-sound pipeline (pkgs/spectrum-ch, pkgs/crosvm-patched,
# pkgs/vhost-device-sound) is x86_64-only.
#
# The matching positive failure case — `graphics.enable = true` or
# `audio.enable = true` on aarch64-linux — is verified by the
# `d2b-platform-gate` block in tests/assertions-eval.sh (not by
# this file; this file's job is to keep the "headless eval passes"
# invariant green).
#
# Returns `system.build.toplevel`, just like the x86_64 smoke test.
# `nix eval --raw -f` on the resulting attribute forces the full
# module-system evaluation but does NOT trigger a build, which is
# important because d2b's x86_64-only build inputs are not
# available on aarch64.
#
# Run via:
#   nix eval --no-write-lock-file -f tests/unit/smoke/smoke-eval-aarch64.nix
#   # or, from the flake:
#   nix-instantiate --eval --strict --expr \
#     "let f = import ./tests/unit/smoke/smoke-eval-aarch64.nix; r = f {}; in r.drvPath"
#
# Wired into tests/static.sh as a Layer-1 gate.

{ pkgs ? import <nixpkgs> { system = "aarch64-linux"; } }:

let
  system = "aarch64-linux";
  inherit (pkgs) lib;

  # Import the flake-as-source via getFlake. Path relative to this
  # file so the test works regardless of caller cwd.
  flake = builtins.getFlake "git+file://${toString ./../../..}";

  # Cross-evaluate by asking the flake's nixpkgs for an aarch64
  # instance directly, rather than relying on `import <nixpkgs>`
  # picking up the consumer's channel. Keeps the test independent of
  # the host's nix-channel state.
  pkgsAarch64 = import flake.inputs.nixpkgs {
    inherit system;
    # Avoid the "package not available on platform" error for any
    # x86_64-only transitive that gets touched outside the
    # graphics/audio gate. The headless smoke path shouldn't hit any,
    # but flipping this to `true` keeps the test resilient.
    config = { allowUnsupportedSystem = true; };
  };

  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;

  nixos = nixosSystem {
    inherit system;
    pkgs = pkgsAarch64;
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        # Minimal NixOS baseline so the eval graph can resolve. Same
        # set as smoke-eval.nix — these knobs aren't exercised by
        # d2b itself, they just make `system.build.toplevel`
        # reachable without a real disk or bootloader.
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

        nixpkgs.hostPlatform = lib.mkForce system;

        # Single consumer-side user that satisfies launcherUsers +
        # ssh.user references. waylandUser is intentionally LEFT
        # UNSET on aarch64 — the cross-domain Wayland forwarding it
        # configures is graphics-coupled, and the assertions schema
        # only requires it when a graphics or audio VM is declared
        # (we declare neither here).
        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };

        d2b.site = {
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
          # Headless workload: graphics and audio remain disabled.
          workloads.headless-vm = {
            providerRefs.runtime = "runtime";
            config = {
              networking.hostName = lib.mkDefault "headless-vm";
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
in
  nixos.config.system.build.toplevel
