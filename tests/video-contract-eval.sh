#!/usr/bin/env bash
# Focused eval contract for the daemon-spawned virtio-media sidecar.
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log() { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok() { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/video-contract-eval.sh"

expr=$(cat <<EOF
let
  flake = builtins.getFlake (toString $ROOT);
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  mkNixos = extra: nixosSystem {
    system = "x86_64-linux";
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        nixpkgs.config.allowUnfree = true;
        boot.loader.grub.enable = false;
        boot.loader.systemd-boot.enable = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
        environment.etc."machine-id".text = "00000000000000000000000000000000";
        system.stateVersion = "25.11";
        users.users.alice = { isNormalUser = true; uid = 1000; };
        nixling.site = {
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };
        nixling.envs.work = {
          lanSubnet = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
        nixling.vms.demo-gfx = {
          enable = true;
          env = "work";
          index = 11;
          ssh.user = "alice";
          graphics.enable = true;
          graphics.videoSidecar = true;
          config = {
            networking.hostName = lib.mkForce "guest-hostname-differs";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
      })
      extra
    ];
  };
  nixos = mkNixos ({ ... }: {});
  processes = nixos.config.nixling._bundle.processesJson.data;
  dag = builtins.head (builtins.filter (vm: vm.vm == "demo-gfx") processes.vms);
  nodeById = id: builtins.head (builtins.filter (node: node.id == id) dag.nodes);
  ch = nodeById "cloud-hypervisor";
  gpu = nodeById "gpu";
  video = nodeById "video";
  videoProfile = nixos.config.nixling._bundle.minijailProfiles."vm-demo-gfx-video".data;
  gpuProfile = nixos.config.nixling._bundle.minijailProfiles."vm-demo-gfx-gpu".data;
  videoUser = nixos.config.users.users."nixling-demo-gfx-video";
  videoGroup = nixos.config.users.groups."nixling-demo-gfx-video";
  mediaPositions =
    builtins.filter (i: builtins.elemAt ch.argv i == "--vhost-user-media")
      (builtins.genList (i: i) (builtins.length ch.argv));
  mediaFlagTokens =
    builtins.filter
      (arg: builtins.isString arg && flake.inputs.nixpkgs.lib.hasPrefix "--vhost-user-media" arg)
      ch.argv;
  expectedMediaArg = "socket=/run/nixling-video/demo-gfx/video.sock";
  renderNixos = mkNixos ({ ... }: {
    nixling.vms.demo-gfx.graphics.renderNodeOnly = true;
  });
  renderDag = builtins.head (builtins.filter (vm: vm.vm == "demo-gfx")
    renderNixos.config.nixling._bundle.processesJson.data.vms);
  nvidiaNixos = mkNixos ({ ... }: {
    nixling.vms.demo-gfx.graphics.videoNvidiaDecode = true;
  });
  nvidiaDag = builtins.head (builtins.filter (vm: vm.vm == "demo-gfx")
    nvidiaNixos.config.nixling._bundle.processesJson.data.vms);
  nvidiaVideo = builtins.head (builtins.filter (node: node.id == "video") nvidiaDag.nodes);
  virglNixos = mkNixos ({ ... }: {
    nixling.vms.demo-gfx.graphics.virglVideo = true;
  });
  virglDag = builtins.head (builtins.filter (vm: vm.vm == "demo-gfx")
    virglNixos.config.nixling._bundle.processesJson.data.vms);
  virglGpu = builtins.head (builtins.filter (node: node.id == "gpu") virglDag.nodes);
  noVideoNixos = mkNixos ({ lib, ... }: {
    nixling.vms.demo-gfx.graphics.videoSidecar = lib.mkForce false;
  });
  failingVideoNoGraphics =
    let bad = mkNixos ({ ... }: {
      nixling.vms.demo-gfx.graphics.enable = flake.inputs.nixpkgs.lib.mkForce false;
      nixling.vms.demo-gfx.graphics.videoSidecar = true;
    });
    in builtins.filter (a: !a.assertion) bad.config.assertions;
  failingNvidiaNoSidecar =
    let bad = mkNixos ({ ... }: {
      nixling.vms.demo-gfx.graphics.videoSidecar = flake.inputs.nixpkgs.lib.mkForce false;
      nixling.vms.demo-gfx.graphics.videoNvidiaDecode = true;
    });
    in builtins.filter (a: !a.assertion) bad.config.assertions;
  failingVirglNoGraphics =
    let bad = mkNixos ({ ... }: {
      nixling.vms.demo-gfx.graphics.enable = flake.inputs.nixpkgs.lib.mkForce false;
      nixling.vms.demo-gfx.graphics.virglVideo = true;
    });
    in builtins.filter (a: !a.assertion) bad.config.assertions;
  failingEqualsMediaOverride =
    let bad = mkNixos ({ ... }: {
      nixling.vms.demo-gfx.config.microvm.cloud-hypervisor.extraArgs = [
        "--vhost-user-media=socket=/run/nixling-video/demo-gfx/evil.sock"
      ];
    });
    in builtins.filter (a: !a.assertion) bad.config.assertions;
in {
  hasVideoNode = video.role == "video";
  videoSocket = builtins.elemAt video.argv 4;
  videoReadiness = video.readiness;
  mediaArgCount = builtins.length mediaPositions;
  mediaFlagTokenCount = builtins.length mediaFlagTokens;
  mediaArg = if mediaPositions == [] then null else builtins.elemAt ch.argv ((builtins.head mediaPositions) + 1);
  defaultVideoDeviceBinds = video.profile.mountPolicy.deviceBinds;
  videoPrivatePidNs = video.profile.namespaces.pid;
  videoPrincipal = videoProfile.principal;
  videoUidMatchesUser = videoProfile.uid == videoUser.uid;
  videoGidMatchesGroup = videoProfile.gid == videoGroup.gid;
  videoUidDiffersFromGpu = videoProfile.uid != gpuProfile.uid;
  videoUserOnlyWithSidecar =
    !(builtins.hasAttr "nixling-demo-gfx-video" noVideoNixos.config.users.users)
    && !(builtins.hasAttr "nixling-demo-gfx-video" noVideoNixos.config.users.groups);
  nvidiaVideoDeviceBinds = nvidiaVideo.profile.mountPolicy.deviceBinds;
  virglDefaultOff = nixos.config.nixling.vms.demo-gfx.graphics.virglVideo == false;
  virglOptInPreservesClosedGpuArgv = builtins.length virglGpu.argv == builtins.length gpu.argv;
  virglDefaultNoStatusMarker =
    !(builtins.any (r: r.kind == "component-specific" && r.value == "graphics.virglVideo=true") gpu.readiness);
  virglStatusMarker =
    builtins.any (r: r.kind == "component-specific" && r.value == "graphics.virglVideo=true") virglGpu.readiness;
  noAlternateMediaArgs =
    builtins.all (arg:
      !(builtins.isString arg && (
        builtins.match ".*tcp=.*" arg != null ||
        builtins.match ".*addr=.*" arg != null ||
        builtins.match ".*port=.*" arg != null ||
        builtins.match ".*vsock.*" arg != null
      ))) (map (i: builtins.elemAt ch.argv (i + 1)) mediaPositions);
  renderHasGpuRenderNode = builtins.any (node: node.id == "gpu-render-node") renderDag.nodes;
  renderVideoEdgeOk = builtins.any (edge: edge.from == "gpu-render-node" && edge.to == "video") renderDag.edges;
  renderNoDanglingGpuEdge = !(builtins.any (edge: edge.from == "gpu" && edge.to == "video") renderDag.edges);
  videoRequiresGraphicsMessage =
    builtins.any (a: builtins.match ".*graphics.videoSidecar requires graphics.enable = true.*" a.message != null)
      failingVideoNoGraphics;
  nvidiaRequiresVideoMessage =
    builtins.any (a: builtins.match ".*graphics.videoNvidiaDecode requires graphics.videoSidecar = true.*" a.message != null)
      failingNvidiaNoSidecar;
  virglRequiresGraphicsMessage =
    builtins.any (a: builtins.match ".*graphics.virglVideo requires graphics.enable = true.*" a.message != null)
      failingVirglNoGraphics;
  equalsMediaOverrideRejected =
    builtins.any (a: builtins.match ".*requires exactly one.*--vhost-user-media.*" a.message != null)
      failingEqualsMediaOverride;
  expectedMediaArg = expectedMediaArg;
}
EOF
)

json=$(nix-instantiate --eval --strict --json --expr "$expr")

jq -e '
  .hasVideoNode == true
  and .videoSocket == "/run/nixling-video/demo-gfx/video.sock"
  and .videoReadiness == [{"kind":"unix-socket-listening","value":"/run/nixling-video/demo-gfx/video.sock"}]
  and .mediaArgCount == 1
  and .mediaFlagTokenCount == 1
  and .mediaArg == .expectedMediaArg
  and .defaultVideoDeviceBinds == ["/dev/dri/renderD128"]
  and .videoPrivatePidNs == true
  and .videoPrincipal == "nixling-demo-gfx-video"
  and .videoUidMatchesUser == true
  and .videoGidMatchesGroup == true
  and .videoUidDiffersFromGpu == true
  and .videoUserOnlyWithSidecar == true
  and (.nvidiaVideoDeviceBinds | sort) == ([ "/dev/dri/renderD128", "/dev/nvidiactl", "/dev/nvidia0", "/dev/nvidia-uvm" ] | sort)
  and .virglDefaultOff == true
  and .virglOptInPreservesClosedGpuArgv == true
  and .virglDefaultNoStatusMarker == true
  and .virglStatusMarker == true
  and .noAlternateMediaArgs == true
  and .renderHasGpuRenderNode == true
  and .renderVideoEdgeOk == true
  and .renderNoDanglingGpuEdge == true
  and .videoRequiresGraphicsMessage == true
  and .nvidiaRequiresVideoMessage == true
  and .virglRequiresGraphicsMessage == true
  and .equalsMediaOverrideRejected == true
' <<<"$json" >/dev/null || {
  printf '%s\n' "$json" | jq . >&2
  fail "video contract eval mismatch"
}

grep -Fq 'let use_video = ${if config.nixling.graphics.virglVideo then "true" else "false"};' \
  "$ROOT/nixos-modules/components/graphics.nix" \
  || fail "graphics.virglVideo is not the source of crosvm/rutabaga use_video"
grep -Fq '.set_use_video(use_video)' \
  "$ROOT/nixos-modules/components/graphics.nix" \
  || fail "crosvm/rutabaga builder does not receive use_video"

ok "video sidecar process graph, dedicated principal, virgl opt-in source path, and assertion contract"
