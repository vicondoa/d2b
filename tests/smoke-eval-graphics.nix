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
# Also asserts the Wave 2 wiring:
#   - guest proxy service uses wl-cross-domain-proxy (not wayland-proxy-virtwl)
#   - no DISPLAY session variable is set (xwayland is unsupported)
#   - host wayland-proxy DAG node is emitted when crossDomainTrusted = true
#   - GPU runner --wayland-sock targets the filter socket, not the real compositor
#   - GPU runner has no XDG_RUNTIME_DIR or WAYLAND_DISPLAY env vars
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

        # A VM with crossDomainTrusted = true to assert that the
        # host-side wayland-proxy DAG node is emitted.
        nixling.vms.demo-cd = {
          enable = true;
          env = "work";
          index = 13;
          ssh.user = "alice";
          graphics.enable = true;
          graphics.crossDomainTrusted = true;
          config = {
            networking.hostName = lib.mkDefault "demo-cd";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };
      })
    ];
  };

  # Guest service assertions for default VM (crossDomainTrusted=false)
  guestServices =
    nixos.config.nixling._computed.demo-gfx.config.systemd.user.services;
  defaultSessionVars =
    nixos.config.nixling._computed.demo-gfx.config.environment.sessionVariables;

  # Guest service assertions for trusted VM (crossDomainTrusted=true)
  trustedGuestServices =
    nixos.config.nixling._computed.demo-cd.config.systemd.user.services;
  trustedProxyExec =
    nixos.config.nixling._computed.demo-cd.config.systemd.user.services.wayland-proxy.serviceConfig.ExecStart;

  # Host DAG node assertions: look for wayland-proxy node in processes bundle
  processes = nixos.config.nixling._bundle.processesJson.data;
  trustedDag = builtins.filter (dag: dag.vm == "demo-cd") processes.vms;
  trustedNodes = if trustedDag == [] then [] else (builtins.head trustedDag).nodes;
  trustedWlproxyNodes = builtins.filter (n: n.id == "wayland-proxy") trustedNodes;

  defaultDag = builtins.filter (dag: dag.vm == "demo-gfx") processes.vms;
  defaultNodes = if defaultDag == [] then [] else (builtins.head defaultDag).nodes;
  defaultWlproxyNodes = builtins.filter (n: n.id == "wayland-proxy") defaultNodes;

  # GPU argv assertions for the trusted VM
  trustedGpuNodes = builtins.filter (n: n.id == "gpu" || n.id == "gpu-render-node") trustedNodes;
  trustedGpuArgv = if trustedGpuNodes == [] then [] else (builtins.head trustedGpuNodes).argv;
  trustedGpuEnv = if trustedGpuNodes == [] then [] else ((builtins.head trustedGpuNodes).env or []);
in
  # Guest proxy: default VM should have no wayland-proxy service (crossDomainTrusted=false)
  assert lib.assertMsg (!(guestServices ? wayland-proxy))
    "default graphics VM (crossDomainTrusted=false) should not have a wayland-proxy guest service";
  # Guest proxy: trusted VM should use wl-cross-domain-proxy (not wayland-proxy-virtwl)
  assert lib.assertMsg (lib.hasInfix "wl-cross-domain-proxy" trustedProxyExec)
    "crossDomainTrusted=true should use wl-cross-domain-proxy in guest proxy service";
  assert lib.assertMsg (!(lib.hasInfix "wayland-proxy-virtwl" trustedProxyExec))
    "crossDomainTrusted=true should NOT use wayland-proxy-virtwl";
  # No Xwayland args in the proxy (xwayland is unsupported)
  assert lib.assertMsg (!(lib.hasInfix "--x-display" trustedProxyExec))
    "proxy service should not include --x-display";
  assert lib.assertMsg (!(lib.hasInfix "--xwayland-binary" trustedProxyExec))
    "proxy service should not include --xwayland-binary";
  # No DISPLAY session variable (xwayland disabled)
  assert lib.assertMsg (!(defaultSessionVars ? DISPLAY))
    "default graphics VM should not set DISPLAY";
  # Host wayland-proxy node: present for crossDomainTrusted=true
  assert lib.assertMsg (builtins.length trustedWlproxyNodes == 1)
    "crossDomainTrusted=true should emit exactly one wayland-proxy host DAG node";
  # Host wayland-proxy node: absent for crossDomainTrusted=false
  assert lib.assertMsg (builtins.length defaultWlproxyNodes == 0)
    "crossDomainTrusted=false should not emit a wayland-proxy host DAG node";
  # GPU argv: --wayland-sock targets the filter socket, not the real compositor
  assert lib.assertMsg (builtins.any (a: lib.hasPrefix "/run/nixling-wlproxy/" a) trustedGpuArgv)
    "GPU runner --wayland-sock should target /run/nixling-wlproxy/<vm>/wayland-0";
  assert lib.assertMsg (!(builtins.any (a: lib.hasPrefix "/run/user/" a) trustedGpuArgv))
    "GPU runner argv should not contain /run/user/<uid> (real compositor path)";
  # GPU env: no XDG_RUNTIME_DIR or WAYLAND_DISPLAY
  assert lib.assertMsg (!(builtins.any (e: lib.hasPrefix "XDG_RUNTIME_DIR=" e) trustedGpuEnv))
    "GPU runner env should not contain XDG_RUNTIME_DIR";
  assert lib.assertMsg (!(builtins.any (e: lib.hasPrefix "WAYLAND_DISPLAY=" e) trustedGpuEnv))
    "GPU runner env should not contain WAYLAND_DISPLAY";
  # Force the readOnly path by strictly evaluating the manifest in
  # addition to the toplevel build. `deepSeq` ensures we don't
  # accept a thunk that lazily skips the manifest assignment.
  builtins.deepSeq nixos.config.nixling.manifest
    nixos.config.system.build.toplevel
