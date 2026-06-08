# tests/smoke-eval-graphics.nix — regression test.
#
# Mirrors tests/smoke-eval.nix but declares ONE graphics-enabled VM.
# Graphics VMs trip the cli.nix `vmLaunchScript` codepath that reads
# `config.nixling.manifest.<name>` directly. That access path is what
# revealed Spec correction #29: when the `nixling.manifest` option
# carried both `readOnly = true` AND `default = { }`, the matching
# `config.nixling.manifest = …` assignment in manifest.nix collided
# with the default and produced "set multiple times" — but ONLY when
# a graphics VM was synthesized. The headless smoke-eval missed it.
#
# Strictly evaluating `config.nixling.manifest` here forces the
# readOnly path and would re-surface a regression of #29 immediately.
#
# Wired into tests/static.sh alongside the existing smoke-eval gate.
{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
}:

let
  inherit (pkgs) lib;

  flake = builtins.getFlake (toString ./..);
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

        nixling.site = {
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };

        nixling.envs.work = {
          lanSubnet    = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };

        # The graphics-enabled VM is the crux of this test — it
        # forces cli.nix's `vmLaunchScript` to dereference
        # `config.nixling.manifest.<name>`, which is the access
        # path that surfaced Spec correction #29.
        nixling.vms.demo-gfx = {
          enable = true;
          env = "work";
          index = 11;
          ssh.user = "alice";
          graphics.enable = true;
          config = {
            networking.hostName = lib.mkDefault "demo-gfx";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };

        nixling.vms.demo-gfx-x = {
          enable = true;
          env = "work";
          index = 12;
          ssh.user = "alice";
          graphics.enable = true;
          graphics.xwayland.enable = true;
          config = {
            networking.hostName = lib.mkDefault "demo-gfx-x";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };
      })
    ];
  };

  defaultProxyExec =
    nixos.config.nixling._computed.demo-gfx.config.systemd.user.services.wayland-proxy.serviceConfig.ExecStart;
  xProxyExec =
    nixos.config.nixling._computed.demo-gfx-x.config.systemd.user.services.wayland-proxy.serviceConfig.ExecStart;
  defaultSessionVars =
    nixos.config.nixling._computed.demo-gfx.config.environment.sessionVariables;
  xSessionVars =
    nixos.config.nixling._computed.demo-gfx-x.config.environment.sessionVariables;
in
  assert lib.assertMsg (!(lib.hasInfix "--x-display" defaultProxyExec))
    "default graphics VM should omit --x-display";
  assert lib.assertMsg (!(lib.hasInfix "--xwayland-binary" defaultProxyExec))
    "default graphics VM should omit --xwayland-binary";
  assert lib.assertMsg (!(defaultSessionVars ? DISPLAY))
    "default graphics VM should not set DISPLAY";
  assert lib.assertMsg (lib.hasInfix "--x-display=0" xProxyExec)
    "graphics.xwayland.enable = true should include --x-display";
  assert lib.assertMsg (lib.hasInfix "--xwayland-binary=" xProxyExec)
    "graphics.xwayland.enable = true should include --xwayland-binary";
  assert lib.assertMsg (xSessionVars.DISPLAY == ":0")
    "graphics.xwayland.enable = true should set DISPLAY";
  # Force the readOnly path by strictly evaluating the manifest in
  # addition to the toplevel build. `deepSeq` ensures we don't
  # accept a thunk that lazily skips the manifest assignment.
  #
  # Spec correction #34: also force the GPU
  # sidecar's serviceConfig.DeviceAllow list so a regression that
  # drops `/dev/net/tun rw` (the v0.1.4 fix for graphics VMs unable
  # to attach to their tap) surfaces here. Cloud-hypervisor needs
  # to open /dev/net/tun + ioctl(TUNSETIFF) on the tap created by
  # upstream microvm.nix's microvm-tap-interfaces@<vm> helper;
  # without the DeviceAllow entry the GPU sidecar crashes early
  # with "Couldn't open /dev/net/tun / Operation not permitted".
  # Spec correction #34 (DEFERRED): the GPU sidecar is no longer a
  # systemd unit. The graphics VM is spawned by the nixling
  # priv-broker as `SpawnRunner{role: Gpu}`, which carries the
  # equivalent `/dev/net/tun` device-cgroup grant in
  # `packages/nixling-priv-broker/src/runners/gpu.rs`. A follow-up
  # broker-gpu-device-allow-eval will re-assert the invariant on
  # the broker side.
  builtins.deepSeq nixos.config.nixling.manifest
    nixos.config.system.build.toplevel
