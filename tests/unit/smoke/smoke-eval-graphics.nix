# tests/unit/smoke/smoke-eval-graphics.nix - regression test.
#
# Mirrors tests/unit/smoke/smoke-eval.nix but declares ONE graphics-enabled VM.
# Graphics VMs trip the cli.nix `vmLaunchScript` codepath that reads
# `config.d2b.manifest.<name>` directly. That access path is what
# revealed Spec correction #29: when the `d2b.manifest` option
# carried both `readOnly = true` AND `default = { }`, the matching
# `config.d2b.manifest = …` assignment in manifest.nix collided
# with the default and produced "set multiple times" - but ONLY when
# a graphics VM was synthesized. The headless smoke-eval missed it.
#
# Strictly evaluating `config.d2b.manifest` here forces the
# readOnly path and would re-surface a regression of #29 immediately.
#
# Also asserts the v3 guest control-plane wiring:
#   - guest frontend is a signed Guest Process node, not a user service
#   - wl-cross-domain-proxy remains in the guest closure
#   - no DISPLAY session variable is set (xwayland is unsupported)
#   - host wayland-proxy DAG node is emitted when crossDomainTrusted = true
#   - GPU runner --wayland-sock targets the filter socket, not the real compositor
#   - GPU runner has no XDG_RUNTIME_DIR or WAYLAND_DISPLAY env vars
#
# Wired into the fixed Bazel Layer-1 graph alongside the smoke-eval gate.
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

        d2b.envs.work = {
          lanSubnet    = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };

        # The graphics-enabled VM is the crux of this test - it
        # forces cli.nix's `vmLaunchScript` to dereference
        # `config.d2b.manifest.<name>`, which is the access
        # path that surfaced Spec correction #29.
        d2b.vms.demo-gfx = {
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
        d2b.vms.demo-cd = {
          enable = true;
          env = "work";
          index = 13;
          ssh.user = "alice";
          graphics.enable = true;
          graphics.crossDomainTrusted = true;
          graphics.waylandFilter = {
            debugLogging = true;
            byteLogging = true;
            denyGlobals = [ "wp_drm_lease_device_v1" ];
            allowGlobals = [ "zwp_linux_dmabuf_v1" ];
            maxVersions.xdg_wm_base = 3;
            dmabufAllow = [ "XR24:linear" ];
            dmabufDeny = [ "all:linear" ];
          };
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
    nixos.config.d2b._computed.demo-gfx.config.systemd.user.services;
  defaultSessionVars =
    nixos.config.d2b._computed.demo-gfx.config.environment.sessionVariables;

  # Host DAG node assertions: look for wayland-proxy node in processes bundle
  processes = nixos.config.d2b._bundle.processesJson.data;
  trustedDag = builtins.filter (dag: dag.vm == "demo-cd") processes.vms;
  trustedDagRecord = if trustedDag == [] then { nodes = [ ]; edges = [ ]; } else builtins.head trustedDag;
  trustedNodes = trustedDagRecord.nodes;
  trustedEdges = trustedDagRecord.edges;
  trustedWlproxyNodes = builtins.filter (n: n.id == "wayland-proxy") trustedNodes;
  trustedWlproxyArgv = if trustedWlproxyNodes == [] then [] else (builtins.head trustedWlproxyNodes).argv;
  trustedWlproxyEnv = if trustedWlproxyNodes == [] then [] else ((builtins.head trustedWlproxyNodes).env or []);
  trustedFrontendNodes = builtins.filter (n: n.id == "wayland-frontend-worker") trustedNodes;
  trustedFrontend = if trustedFrontendNodes == [] then { } else builtins.head trustedFrontendNodes;
  trustedGuestPackages = nixos.config.d2b._computed.demo-cd.config.environment.systemPackages;
  hasArgPair = argv: flag: value:
    let len = builtins.length argv;
    in len >= 2 && builtins.any
      (i: builtins.elemAt argv i == flag && builtins.elemAt argv (i + 1) == value)
      (lib.range 0 (len - 2));

  defaultDag = builtins.filter (dag: dag.vm == "demo-gfx") processes.vms;
  defaultDagRecord = if defaultDag == [] then { nodes = [ ]; edges = [ ]; } else builtins.head defaultDag;
  defaultNodes = defaultDagRecord.nodes;
  defaultEdges = defaultDagRecord.edges;
  defaultWlproxyNodes = builtins.filter (n: n.id == "wayland-proxy") defaultNodes;

  # GPU argv assertions for the trusted VM
  trustedGpuNodes = builtins.filter (n: n.id == "gpu" || n.id == "gpu-render-node") trustedNodes;
  trustedGpuArgv = if trustedGpuNodes == [] then [] else (builtins.head trustedGpuNodes).argv;
  trustedGpuEnv = if trustedGpuNodes == [] then [] else ((builtins.head trustedGpuNodes).env or []);
  trustedGraphicsNodeId = if trustedGpuNodes == [] then "" else (builtins.head trustedGpuNodes).id;

  # GPU argv assertions for default VM (crossDomainTrusted=false)
  defaultGpuNodes = builtins.filter (n: n.id == "gpu" || n.id == "gpu-render-node") defaultNodes;
  defaultGpuArgv = if defaultGpuNodes == [] then [] else (builtins.head defaultGpuNodes).argv;
  defaultGpuEnv = if defaultGpuNodes == [] then [] else ((builtins.head defaultGpuNodes).env or []);
in
  # Guest frontend: no direct user service remains for either VM.
  assert lib.assertMsg (!(guestServices ? wayland-proxy))
    "default graphics VM (crossDomainTrusted=false) should not have a wayland-proxy guest service";
  assert lib.assertMsg (!(nixos.config.d2b._computed.demo-cd.config.systemd.user.services ? wayland-proxy))
    "trusted graphics VM should not have a wayland-proxy guest service";
  assert lib.assertMsg (builtins.length trustedFrontendNodes == 1)
    "crossDomainTrusted=true should emit exactly one Guest frontend Process node";
  assert lib.assertMsg (trustedFrontend.executionRef == "Guest/demo-cd")
    "Guest frontend Process should target the demo-cd guest execution reference";
  assert lib.assertMsg (trustedFrontend.executionDomain == "system")
    "Guest frontend Process should use the system execution domain";
  assert lib.assertMsg (lib.hasInfix "wl-cross-domain-proxy" trustedFrontend.binaryPath)
    "Guest frontend Process should use wl-cross-domain-proxy";
  assert lib.assertMsg (hasArgPair trustedFrontend.argv "--socket-name" "wayland-1")
    "Guest frontend Process should bind the guest desktop wayland-1 socket";
  assert lib.assertMsg (builtins.any
    (pkg: lib.hasInfix "wl-cross-domain-proxy" (pkg.name or ""))
    trustedGuestPackages)
    "trusted graphics VM should retain wl-cross-domain-proxy in its guest closure";
  # No DISPLAY session variable (xwayland disabled)
  assert lib.assertMsg (!(defaultSessionVars ? DISPLAY))
    "default graphics VM should not set DISPLAY";
  # Host wayland-proxy node: present for crossDomainTrusted=true
  assert lib.assertMsg (builtins.length trustedWlproxyNodes == 1)
    "crossDomainTrusted=true should emit exactly one wayland-proxy host DAG node";
  # Host wayland-proxy node: absent for crossDomainTrusted=false
  assert lib.assertMsg (builtins.length defaultWlproxyNodes == 0)
    "crossDomainTrusted=false should not emit a wayland-proxy host DAG node";
  assert lib.assertMsg (!(builtins.any (e: e.from == "wayland-proxy" || e.to == "wayland-proxy") defaultEdges))
    "crossDomainTrusted=false should not emit wayland-proxy DAG edges";
  assert lib.assertMsg (builtins.any (a: lib.hasPrefix "/run/user/1000/" a) defaultGpuArgv)
    "default GPU runner should use the real host compositor socket when the filter proxy is absent";
  assert lib.assertMsg (!(builtins.any (a: lib.hasPrefix "/run/d2b-wlproxy/" a) defaultGpuArgv))
    "default GPU runner should not target the filter socket when no filter proxy node exists";
  # GPU argv: --wayland-sock targets the filter socket, not the real compositor
  assert lib.assertMsg (builtins.any (a: lib.hasPrefix "/run/d2b-wlproxy/" a) trustedGpuArgv)
    "GPU runner --wayland-sock should target /run/d2b-wlproxy/<vm>/wayland-0";
  assert lib.assertMsg (!(builtins.any (a: lib.hasPrefix "/run/user/" a) trustedGpuArgv))
    "GPU runner argv should not contain /run/user/<uid> (real compositor path)";
  assert lib.assertMsg (builtins.any (e: e.from == "wayland-proxy" && e.to == trustedGraphicsNodeId) trustedEdges)
    "trusted graphics DAG should contain wayland-proxy -> graphicsNodeId edge";
  assert lib.assertMsg (hasArgPair trustedWlproxyArgv "--deny-global" "wp_drm_lease_device_v1")
    "waylandFilter.denyGlobals should serialize to wayland-proxy argv";
  assert lib.assertMsg (hasArgPair trustedWlproxyArgv "--allow-global" "zwp_linux_dmabuf_v1")
    "waylandFilter.allowGlobals should serialize to wayland-proxy argv";
  assert lib.assertMsg (hasArgPair trustedWlproxyArgv "--max-version" "xdg_wm_base=3")
    "waylandFilter.maxVersions should serialize to wayland-proxy argv";
  assert lib.assertMsg (hasArgPair trustedWlproxyArgv "--dmabuf-allow" "XR24:linear")
    "waylandFilter.dmabufAllow should serialize to wayland-proxy argv";
  assert lib.assertMsg (hasArgPair trustedWlproxyArgv "--dmabuf-deny" "all:linear")
    "waylandFilter.dmabufDeny should serialize to wayland-proxy argv";
  assert lib.assertMsg (builtins.elem "WL_PROXY_DEBUG=1" trustedWlproxyEnv && builtins.elem "WL_PROXY_PREFIX=d2b-demo-cd-wlproxy" trustedWlproxyEnv)
    "waylandFilter.debugLogging should serialize WL_PROXY_DEBUG/WL_PROXY_PREFIX to wayland-proxy env";
  assert lib.assertMsg (builtins.elem "WL_PROXY_HEXDUMP=1" trustedWlproxyEnv && builtins.elem "WL_PROXY_HEXDUMP_LIMIT=256" trustedWlproxyEnv)
    "waylandFilter.byteLogging should serialize WL_PROXY_HEXDUMP/WL_PROXY_HEXDUMP_LIMIT to wayland-proxy env";
  assert lib.assertMsg (hasArgPair trustedWlproxyArgv "--listen" "/run/d2b-wlproxy/demo-cd/wayland-0")
    "wayland-proxy argv should listen on the filter socket used by readiness";
  assert lib.assertMsg (hasArgPair trustedWlproxyArgv "--connect" "/run/user/1000/wayland-0")
    "wayland-proxy argv should connect to the real host compositor path";
  # GPU env: no XDG_RUNTIME_DIR or WAYLAND_DISPLAY
  assert lib.assertMsg (!(builtins.any (e: lib.hasPrefix "XDG_RUNTIME_DIR=" e) trustedGpuEnv))
    "GPU runner env should not contain XDG_RUNTIME_DIR";
  assert lib.assertMsg (!(builtins.any (e: lib.hasPrefix "WAYLAND_DISPLAY=" e) trustedGpuEnv))
    "GPU runner env should not contain WAYLAND_DISPLAY";
  assert lib.assertMsg (!(builtins.any (e: lib.hasPrefix "XDG_RUNTIME_DIR=" e) defaultGpuEnv))
    "default GPU runner env should not contain XDG_RUNTIME_DIR";
  assert lib.assertMsg (!(builtins.any (e: lib.hasPrefix "WAYLAND_DISPLAY=" e) defaultGpuEnv))
    "default GPU runner env should not contain WAYLAND_DISPLAY";
  # Force the readOnly path by strictly evaluating the manifest in
  # addition to the toplevel build. `deepSeq` ensures we don't
  # accept a thunk that lazily skips the manifest assignment.
  builtins.deepSeq nixos.config.d2b.manifest
    nixos.config.system.build.toplevel
