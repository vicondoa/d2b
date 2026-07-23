# nix-unit cases migrated from tests/video-contract-eval.sh.
#
# Focused eval contract for the daemon-spawned virtio-media video sidecar
# (`graphics.videoSidecar = true`): the per-VM process DAG carries a `video`
# node listening on the canonical AF_UNIX socket, Cloud Hypervisor gets
# exactly one `--vhost-user-media socket=...` argument (no tcp/addr/port/
# vsock alternative), the video runner uses its own dedicated principal
# (distinct UID/GID from the GPU runner, materialised only when the sidecar
# is enabled), the default device-bind allowlist is renderD128-only and
# expands to the NVIDIA decode nodes under `videoNvidiaDecode`, the
# `renderNodeOnly` GPU split re-points the DAG edge at `gpu-render-node`,
# the experimental `virglVideo` GPU path is default-off and toggles the
# component-specific readiness marker without reshaping the closed-GPU
# argv, and the four guard assertions reject the misconfigurations they are
# meant to catch.
#
# The two source-content checks the bash gate ran with `grep -Fq` against
# `nixos-modules/components/graphics.nix` (that `graphics.virglVideo` is the
# source of crosvm/rutabaga `use_video`) migrate to `lib.hasInfix` over
# `builtins.readFile` of the same file — a value assertion.
#
# Faithful note on the assertion-message checks: the bash gate used
# `builtins.match ".*<msg>.*"`, which is fragile across the multi-line
# assertion messages (the `.` newline behaviour is implementation-defined).
# These cases use `lib.hasInfix` with the single-line contiguous substring
# that actually appears in each message instead.
#
# This gate synthesises a graphics + video VM, which the framework's
# checkVmPlatform gate refuses on aarch64. The bash gate hardcoded
# system = "x86_64-linux"; mirror that by contributing these cases only to
# the x86_64-linux nix-unit check.
{ mkEval, lib, system, flakeRoot, ... }:

lib.optionalAttrs (system == "x86_64-linux") (

let
  base = { lib, ... }: {
    nixpkgs.config.allowUnfree = true;
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    d2b.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    d2b.vms.demo-gfx = {
      enable = true;
      env = "work";
      index = 11;
      ssh.user = "alice";
      graphics.enable = true;
      graphics.videoSidecar = true;
      config = { lib, ... }: {
        networking.hostName = lib.mkForce "guest-hostname-differs";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
  };

  mk = extra: mkEval [ base extra ];
  nixos = mk ({ ... }: { });

  processes = nixos.config.d2b._bundle.processesJson.data;
  dag = builtins.head (builtins.filter (vm: vm.vm == "demo-gfx") processes.vms);
  nodeById = id: builtins.head (builtins.filter (node: node.id == id) dag.nodes);
  ch = nodeById "cloud-hypervisor";
  gpu = nodeById "gpu";
  video = nodeById "video";

  videoProfile = nixos.config.d2b._bundle.minijailProfiles."vm-demo-gfx-video".data;
  gpuProfile = nixos.config.d2b._bundle.minijailProfiles."vm-demo-gfx-gpu".data;
  videoUser = nixos.config.users.users."d2b-demo-gfx-video";
  videoGroup = nixos.config.users.groups."d2b-demo-gfx-video";

  mediaPositions =
    builtins.filter (i: builtins.elemAt ch.argv i == "--vhost-user-media")
      (builtins.genList (i: i) (builtins.length ch.argv));
  mediaFlagTokens =
    builtins.filter
      (arg: builtins.isString arg && lib.hasPrefix "--vhost-user-media" arg)
      ch.argv;
  mediaArgValues = map (i: builtins.elemAt ch.argv (i + 1)) mediaPositions;
  expectedMediaArg = "socket=/run/d2b-video/demo-gfx/video.sock";

  renderNixos = mk ({ ... }: {
    d2b.vms.demo-gfx.graphics.renderNodeOnly = true;
  });
  renderDag = builtins.head (builtins.filter (vm: vm.vm == "demo-gfx")
    renderNixos.config.d2b._bundle.processesJson.data.vms);

  nvidiaNixos = mk ({ ... }: {
    d2b.vms.demo-gfx.graphics.videoNvidiaDecode = true;
  });
  nvidiaDag = builtins.head (builtins.filter (vm: vm.vm == "demo-gfx")
    nvidiaNixos.config.d2b._bundle.processesJson.data.vms);
  nvidiaVideo = builtins.head (builtins.filter (node: node.id == "video") nvidiaDag.nodes);

  virglNixos = mk ({ ... }: {
    d2b.vms.demo-gfx.graphics.virglVideo = true;
  });
  virglDag = builtins.head (builtins.filter (vm: vm.vm == "demo-gfx")
    virglNixos.config.d2b._bundle.processesJson.data.vms);
  virglGpu = builtins.head (builtins.filter (node: node.id == "gpu") virglDag.nodes);

  noVideoNixos = mk ({ lib, ... }: {
    d2b.vms.demo-gfx.graphics.videoSidecar = lib.mkForce false;
  });

  failingAssertions = cfg: builtins.filter (a: !a.assertion) cfg.config.assertions;
  anyMsg = sub: assertions: builtins.any (a: lib.hasInfix sub a.message) assertions;

  failingVideoNoGraphics = failingAssertions (mk ({ ... }: {
    d2b.vms.demo-gfx.graphics.enable = lib.mkForce false;
    d2b.vms.demo-gfx.graphics.videoSidecar = true;
  }));
  failingNvidiaNoSidecar = failingAssertions (mk ({ ... }: {
    d2b.vms.demo-gfx.graphics.videoSidecar = lib.mkForce false;
    d2b.vms.demo-gfx.graphics.videoNvidiaDecode = true;
  }));
  failingVirglNoGraphics = failingAssertions (mk ({ ... }: {
    d2b.vms.demo-gfx.graphics.enable = lib.mkForce false;
    d2b.vms.demo-gfx.graphics.virglVideo = true;
  }));
  failingEqualsMediaOverride = failingAssertions (mk ({ ... }: {
    d2b.vms.demo-gfx.config.microvm.cloud-hypervisor.extraArgs = [
      "--vhost-user-media=socket=/run/d2b-video/demo-gfx/evil.sock"
    ];
  }));

  graphicsSrc = builtins.readFile (flakeRoot + "/nixos-modules/components/graphics.nix");
  useVideoLine = ''let use_video = ''${if config.d2b.graphics.virglVideo then "true" else "false"};'';
  setUseVideoCall = ".set_use_video(use_video)";
in
{
  "video-contract/has-video-node" = {
    expr = video.role;
    expected = "video";
  };
  "video-contract/video-socket" = {
    expr = builtins.elemAt video.argv 4;
    expected = "/run/d2b-video/demo-gfx/video.sock";
  };
  "video-contract/video-readiness" = {
    expr = video.readiness;
    expected = [
      { kind = "unix-socket-listening"; value = "/run/d2b-video/demo-gfx/video.sock"; }
    ];
  };
  "video-contract/media-arg-count" = {
    expr = builtins.length mediaPositions;
    expected = 1;
  };
  "video-contract/media-flag-token-count" = {
    expr = builtins.length mediaFlagTokens;
    expected = 1;
  };
  "video-contract/media-arg-value" = {
    expr = builtins.elemAt ch.argv ((builtins.head mediaPositions) + 1);
    expected = expectedMediaArg;
  };
  "video-contract/no-alternate-media-args" = {
    expr = builtins.all
      (arg: !(builtins.isString arg && (
        lib.hasInfix "tcp=" arg
        || lib.hasInfix "addr=" arg
        || lib.hasInfix "port=" arg
        || lib.hasInfix "vsock" arg)))
      mediaArgValues;
    expected = true;
  };
  "video-contract/default-video-device-binds" = {
    expr = video.profile.mountPolicy.deviceBinds;
    expected = [ "/dev/dri/renderD128" ];
  };
  "video-contract/video-private-pid-ns" = {
    expr = video.profile.namespaces.pid;
    expected = true;
  };
  "video-contract/video-principal" = {
    expr = videoProfile.principal;
    expected = "d2b-demo-gfx-video";
  };
  "video-contract/video-uid-matches-user" = {
    expr = videoProfile.uid == videoUser.uid;
    expected = true;
  };
  "video-contract/video-gid-matches-group" = {
    expr = videoProfile.gid == videoGroup.gid;
    expected = true;
  };
  "video-contract/video-uid-differs-from-gpu" = {
    expr = videoProfile.uid != gpuProfile.uid;
    expected = true;
  };
  "video-contract/video-user-only-with-sidecar" = {
    expr =
      !(builtins.hasAttr "d2b-demo-gfx-video" noVideoNixos.config.users.users)
      && !(builtins.hasAttr "d2b-demo-gfx-video" noVideoNixos.config.users.groups);
    expected = true;
  };
  "video-contract/nvidia-video-device-binds" = {
    expr = builtins.sort builtins.lessThan nvidiaVideo.profile.mountPolicy.deviceBinds;
    expected = builtins.sort builtins.lessThan [
      "/dev/dri/renderD128"
      "/dev/nvidiactl"
      "/dev/nvidia0"
      "/dev/nvidia-uvm"
    ];
  };
  "video-contract/virgl-default-off" = {
    expr = nixos.config.d2b.vms.demo-gfx.graphics.virglVideo;
    expected = false;
  };
  "video-contract/virgl-opt-in-preserves-closed-gpu-argv" = {
    expr = builtins.length virglGpu.argv == builtins.length gpu.argv;
    expected = true;
  };
  "video-contract/virgl-default-no-status-marker" = {
    expr = builtins.any
      (r: r.kind == "component-specific" && r.value == "graphics.virglVideo=true")
      gpu.readiness;
    expected = false;
  };
  "video-contract/virgl-status-marker" = {
    expr = builtins.any
      (r: r.kind == "component-specific" && r.value == "graphics.virglVideo=true")
      virglGpu.readiness;
    expected = true;
  };
  "video-contract/render-has-gpu-render-node" = {
    expr = builtins.any (node: node.id == "gpu-render-node") renderDag.nodes;
    expected = true;
  };
  "video-contract/render-video-edge-ok" = {
    expr = builtins.any (edge: edge.from == "gpu-render-node" && edge.to == "video") renderDag.edges;
    expected = true;
  };
  "video-contract/render-no-dangling-gpu-edge" = {
    expr = builtins.any (edge: edge.from == "gpu" && edge.to == "video") renderDag.edges;
    expected = false;
  };
  "video-contract/video-requires-graphics-message" = {
    expr = anyMsg "graphics.videoSidecar requires graphics.enable = true." failingVideoNoGraphics;
    expected = true;
  };
  "video-contract/nvidia-requires-video-message" = {
    expr = anyMsg "graphics.videoNvidiaDecode requires graphics.videoSidecar = true." failingNvidiaNoSidecar;
    expected = true;
  };
  "video-contract/virgl-requires-graphics-message" = {
    expr = anyMsg "graphics.virglVideo requires graphics.enable = true." failingVirglNoGraphics;
    expected = true;
  };
  "video-contract/equals-media-override-rejected" = {
    expr = anyMsg "--vhost-user-media argument equal to socket=/run/d2b-video/demo-gfx/video.sock" failingEqualsMediaOverride;
    expected = true;
  };
  "video-contract/graphics-virgl-use-video-source" = {
    expr = lib.hasInfix useVideoLine graphicsSrc;
    expected = true;
  };
  "video-contract/graphics-set-use-video-source" = {
    expr = lib.hasInfix setUseVideoCall graphicsSrc;
    expected = true;
  };
}
)
