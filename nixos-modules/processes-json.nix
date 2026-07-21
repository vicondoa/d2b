{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  workloads = import ./workload-process-rows.nix {
    inherit config lib pkgs;
  };
  roles = import ./role-process-rows.nix {
    inherit config lib pkgs;
  };
  audioRows = import ./realm-audio-rows.nix {
    inherit config lib pkgs;
  };
  d2bChVsockConnect = import ./d2b-ch-vsock-connect.nix { inherit pkgs; };

  clean = value: lib.strings.sanitizeDerivationName value;
  resource = workloadId: kind:
    lib.findFirst
      (row: row.kind == kind)
      (throw "workload ${workloadId} is missing normalized ${kind}")
      (cfg._index.resources.byWorkloadId.${workloadId} or [ ]);
  roleFor = workloadId: kind:
    lib.findFirst
      (row: row.workloadId == workloadId && row.roleKind == kind)
      null
      roles;
  roleRuntime = role:
    (lib.findFirst
      (row: row.kind == "role-runtime")
      (throw "role ${role.roleId} is missing normalized role-runtime")
      (cfg._index.resources.byRoleId.${role.roleId} or [ ])).path;
  displayMappingFor = workloadId:
    let
      mappings = lib.filter
        (mapping: mapping.workloadId == workloadId)
        cfg._index.providerRegistryV2Mappings.display;
    in
    if builtins.length mappings == 1
    then builtins.head mappings
    else throw
      "workload ${workloadId} must have exactly one normalized display mapping";
  displayEndpointFor = workloadId: endpointKind:
    let
      mapping = displayMappingFor workloadId;
      ownerRole = roleFor workloadId "wayland-proxy";
      resourceId = mapping.endpointIds.${endpointKind};
      endpoint = cfg._index.resources.byId.${resourceId}
        or (throw
          "workload ${workloadId} display mapping references an unknown ${endpointKind} endpoint");
      expectedKind = "display-endpoint-${
        if endpointKind == "crossDomain" then "cross-domain" else endpointKind
      }";
    in
    if ownerRole == null
      || ownerRole.roleId != mapping.ownerRoleId
      || endpoint.kind != expectedKind
      || endpoint.providerId != mapping.providerId
      || endpoint.realmId != mapping.realmId
      || endpoint.workloadId != mapping.workloadId
      || endpoint.roleId != mapping.ownerRoleId
    then throw
      "workload ${workloadId} ${endpointKind} endpoint disagrees with normalized display authority"
    else endpoint;
  waylandSocketFor = workloadId:
    let endpoint = displayEndpointFor workloadId "wayland";
    in
    if endpoint.path == null
    then throw "workload ${workloadId} normalized Wayland endpoint has no socket path"
    else endpoint.path;
  profile = nodeId:
    cfg._bundle.minijailProfiles."role-${nodeId}".roleProfile;
  audioFor = workloadId:
    lib.findFirst
      (row: row.workloadId == workloadId)
      (throw "workload ${workloadId} is missing its canonical audio process")
      audioRows.processes;
  audioEndpointFor = workloadId:
    lib.findFirst
      (row: row.workloadId == workloadId)
      (throw "workload ${workloadId} is missing its canonical audio endpoint")
      audioRows.endpoints;

  readiness = kind: value: { inherit kind value; };
  socketExists = path: readiness "unix-socket-exists" path;
  socketListening = path: readiness "unix-socket-listening" path;
  componentReady = value: readiness "component-specific" value;
  commandReady = value: readiness "command" value;

  mkNode =
    {
      id,
      role,
      ready ? [ ],
      binaryPath ? null,
      argv ? [ ],
      env ? [ ],
      planOps ? [ ],
      networkInterfaces ? [ ],
    }:
    assert (binaryPath == null) == (argv == [ ]);
    {
      inherit id role;
      readiness = ready;
      profile = profile id;
    }
    // lib.optionalAttrs (binaryPath != null) {
      inherit binaryPath argv;
    }
    // lib.optionalAttrs (env != [ ]) { inherit env; }
    // lib.optionalAttrs (planOps != [ ]) { inherit planOps; }
    // lib.optionalAttrs (networkInterfaces != [ ]) {
      inherit networkInterfaces;
    };

  edge = from: to: reason: { inherit from to reason; };

  threadPoolSize = microvm:
    let raw = microvm.virtiofsd.threadPoolSize;
    in
    if builtins.isInt raw then toString raw
    else if builtins.isString raw && builtins.match "^[0-9]+$" raw != null
    then raw
    else toString (lib.max 1 microvm.vcpu);

  variadic = flag: values:
    lib.optionals (values != [ ]) ([ flag ] ++ values);

  volumePath = workload: volume:
    if lib.hasPrefix "/" volume.image
    then volume.image
    else "${workload.stateRoot}/volumes/${volume.image}";

  cloudHypervisorArgv = workload: microvm:
    let
      cloud = roleFor workload.workloadId "cloud-hypervisor";
      apiSocket = "${roleRuntime cloud}/api.sock";
      kernelParams = lib.concatStringsSep " " (
        [
          (if pkgs.stdenv.hostPlatform.system == "x86_64-linux"
           then "earlyprintk=ttyS0 console=ttyS0"
           else "console=ttyAMA0")
          "reboot=t"
          "panic=-1"
        ]
        ++ microvm.kernelParams
      );
      diskParams = map
        (volume:
          let
            base = "path=${volumePath workload volume}";
            readonly = if volume.readOnly or false then ",readonly=on" else "";
          in
          "${base}${readonly}")
        microvm.volumes;
      fsParams = map
        (share:
          let
            virtiofs = roleFor workload.workloadId "virtiofsd";
            socket = "${roleRuntime virtiofs}/${clean share.tag}.sock";
          in
          "tag=${share.tag},socket=${socket}")
        workload.shares;
      # Non-macvtap ("tap") interfaces plug straight into `--net tap=...`.
      # macvtap interfaces are broker-provisioned: the broker resolves each
      # macvtap-typed entry in `network_interfaces` (in list order) to an
      # inherited fd starting at 10 (see `resolve_macvtap_intents` /
      # `RENDER_NODE_INHERITED_FD` in d2b-priv-broker) and hands the runner
      # that fd already opened, so the argv side only needs to name it.
      netInterfaceArg =
        let
          nextMacvtapFd = index:
            10 + builtins.length
              (lib.filter (iface: iface.type == "macvtap")
                (lib.sublist 0 index workload.networkInterfaces));
        in
        index: iface:
          if iface.type == "macvtap"
          then "fd=${toString (nextMacvtapFd index)},mac=${iface.mac}"
          else "tap=${iface.id},mac=${iface.mac}";
      netParams = lib.imap0 netInterfaceArg workload.networkInterfaces;
      tpm = roleFor workload.workloadId "swtpm";
      gpu = roleFor workload.workloadId "gpu";
      gpuRender = roleFor workload.workloadId "gpu-render-node";
      activeGpu =
        if microvm.graphics.renderNodeOnly or false then gpuRender else gpu;
      video = roleFor workload.workloadId "video";
      audio = roleFor workload.workloadId "audio";
    in
    [
      "microvm@${workload.workloadId}"
      "--cpus" "boot=${toString microvm.vcpu}"
      "--watchdog"
      "--kernel"
      (if pkgs.stdenv.hostPlatform.system == "x86_64-linux"
       then "${microvm.kernel.dev}/vmlinux"
       else "${microvm.kernel.out}/${pkgs.stdenv.hostPlatform.linux-kernel.target}")
      "--initramfs" (toString microvm.initrdPath)
      "--cmdline" kernelParams
      "--seccomp" "true"
      "--memory" "size=${toString microvm.mem}M,shared=on"
      "--platform"
      "oem_strings=[io.systemd.credential:vmm.notify_socket=vsock-stream:2:8888]"
      "--console" "null"
      "--serial" "tty"
      "--vsock" "cid=${toString microvm.vsock.cid},socket=${microvm.vsock.socket}"
    ]
    ++ variadic "--disk" diskParams
    ++ variadic "--fs" fsParams
    ++ [ "--api-socket" apiSocket ]
    ++ variadic "--net" netParams
    ++ lib.optionals (tpm != null) [
      "--tpm" "socket=${roleRuntime tpm}/tpm.sock"
    ]
    ++ lib.optionals (activeGpu != null) [
      "--gpu" "socket=${roleRuntime activeGpu}/gpu.sock"
    ]
    ++ lib.optionals (video != null) [
      "--vhost-user-media" "socket=${roleRuntime video}/video.sock"
    ]
    ++ lib.optionals (audio != null) [
      "--generic-vhost-user"
      "socket=${(audioEndpointFor workload.workloadId).path},virtio_id=25,queue_sizes=[64,64,64,64]"
    ]
    ++ microvm.cloud-hypervisor.extraArgs;

  virtiofsNodes = workload: microvm:
    let role = roleFor workload.workloadId "virtiofsd";
    in
    if role == null then [ ] else map
      (share:
        let
          id = "${role.roleId}-${share.tag}";
          source = share.servedSource or share.source;
          readOnly =
            share.tag == "ro-store"
            || share.tag == "d2b-meta"
            || (share.readOnly or false);
          socket = "${roleRuntime role}/${clean share.tag}.sock";
        in
        mkNode {
          inherit id;
          role = "virtiofsd";
          ready = [ (socketListening socket) ];
          binaryPath = "${microvm.virtiofsd.package}/bin/virtiofsd";
          argv = [
            "microvm-virtiofsd@${workload.workloadId}-${clean share.tag}"
            "--socket-path=${socket}"
            "--shared-dir=${source}"
            "--thread-pool-size" (threadPoolSize microvm)
            "--sandbox=chroot"
            "--inode-file-handles=never"
            "--cache=${share.cache or "auto"}"
          ]
          ++ lib.optionals (microvm.virtiofsd.group != null) [
            "--socket-group=${microvm.virtiofsd.group}"
          ]
          ++ lib.optional readOnly "--readonly"
          ++ microvm.virtiofsd.extraArgs;
        })
      workload.shares;

  swtpmFlushScript = workload: role:
    pkgs.writeShellScript "d2b-swtpm-flush-${workload.workloadId}" ''
      set -eu
      state_dir=${lib.escapeShellArg "${workload.stateRoot}/tpm"}
      permall_file="$state_dir/swtpm_perm.state"
      flush_sock=${lib.escapeShellArg "${roleRuntime role}/tpm-flush.sock"}

      if [ ! -f "$permall_file" ]; then
        exit 0
      fi

      cleanup() {
        ${pkgs.coreutils}/bin/rm -f -- "$flush_sock"
        if [ -n "''${swtpm_pid:-}" ] && kill -0 "$swtpm_pid" 2>/dev/null; then
          kill "$swtpm_pid" 2>/dev/null || true
          wait "$swtpm_pid" 2>/dev/null || true
        fi
      }
      trap cleanup EXIT

      ${pkgs.swtpm}/bin/swtpm socket \
        --tpmstate dir="$state_dir" \
        --ctrl type=unixio,path="$flush_sock",mode=0600 \
        --tpm2 \
        --flags startup-clear &
      swtpm_pid=$!

      for _ in $(${pkgs.coreutils}/bin/seq 1 50); do
        if [ -S "$flush_sock" ]; then
          break
        fi
        ${pkgs.coreutils}/bin/sleep 0.1
      done

      [ -S "$flush_sock" ]
      ${pkgs.swtpm}/bin/swtpm_ioctl --unix "$flush_sock" -i
      ${pkgs.swtpm}/bin/swtpm_ioctl --unix "$flush_sock" -s
      wait "$swtpm_pid"
    '';

  d2bWaylandProxy = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b-wayland-proxy";
    version = "2.0.0";
    src = d2bLib.cleanRustPackagesSource ../packages;
    cargoLock = {
      lockFile = ../packages/Cargo.lock;
      outputHashes."wl-proxy-0.1.2" =
        "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
    };
    cargoBuildFlags = [ "--package" "d2b-wayland-proxy" ];
    doCheck = false;
    postPatch = ''
      mkdir -p .cargo
      cat > .cargo/config.toml <<EOF
[build]
rustc-wrapper = ""
EOF
      rm -f .cargo/rustc-wrapper.sh
    '';
    installPhase = ''
      runHook preInstall
      install -Dm755 target/x86_64-unknown-linux-gnu/release/d2b-wayland-proxy \
        $out/bin/d2b-wayland-proxy 2>/dev/null \
        || install -Dm755 target/release/d2b-wayland-proxy \
          $out/bin/d2b-wayland-proxy
      runHook postInstall
    '';
  };

  crosvmVideo = pkgs.crosvm.overrideAttrs (old: {
    buildInputs = (old.buildInputs or [ ]) ++ [ pkgs.libva ];
    cargoBuildFeatures =
      (old.cargoBuildFeatures or old.buildFeatures or [ ])
      ++ [ "video-decoder" "vaapi" "media" ];
    cargoCheckFeatures =
      (old.cargoCheckFeatures or old.cargoBuildFeatures
        or old.buildFeatures or [ ])
      ++ [ "video-decoder" "vaapi" "media" ];
    postPatch = (old.postPatch or "") + ''
      mkdir -p devices/src/virtio/vhost_user_backend/video/sys
      cp ${../pkgs/vhost-user-video/mod.rs} devices/src/virtio/vhost_user_backend/video/mod.rs
      cp ${../pkgs/vhost-user-video/sys_mod.rs} devices/src/virtio/vhost_user_backend/video/sys/mod.rs
      cp ${../pkgs/vhost-user-video/sys_linux.rs} devices/src/virtio/vhost_user_backend/video/sys/linux.rs
      substituteInPlace devices/src/virtio/vhost_user_backend/mod.rs \
        --replace-fail '#[cfg(feature = "audio")]
pub mod snd;' '#[cfg(feature = "audio")]
pub mod snd;
#[cfg(feature = "video-decoder")]
pub mod video;'
      substituteInPlace devices/src/virtio/vhost_user_backend/mod.rs \
        --replace-fail '#[cfg(feature = "audio")]
pub use snd::run_snd_device;
#[cfg(feature = "audio")]
pub use snd::Options as SndOptions;' '#[cfg(feature = "audio")]
pub use snd::run_snd_device;
#[cfg(feature = "audio")]
pub use snd::Options as SndOptions;
#[cfg(feature = "video-decoder")]
pub use video::run_video_device;
#[cfg(feature = "video-decoder")]
pub use video::Options as VideoOptions;'
      substituteInPlace src/crosvm/cmdline.rs \
        --replace-fail '#[cfg(feature = "audio")]
    Snd(vhost_user_backend::SndOptions),' '#[cfg(feature = "audio")]
    Snd(vhost_user_backend::SndOptions),
    #[cfg(feature = "video-decoder")]
    Video(vhost_user_backend::VideoOptions),'
      substituteInPlace src/main.rs \
        --replace-fail '#[cfg(feature = "audio")]
use devices::virtio::vhost_user_backend::run_snd_device;' '#[cfg(feature = "audio")]
use devices::virtio::vhost_user_backend::run_snd_device;
#[cfg(feature = "video-decoder")]
use devices::virtio::vhost_user_backend::run_video_device;'
      substituteInPlace src/main.rs \
        --replace-fail '#[cfg(feature = "audio")]
            CrossPlatformDevicesCommands::Snd(cfg) => run_snd_device(cfg),' '#[cfg(feature = "audio")]
            CrossPlatformDevicesCommands::Snd(cfg) => run_snd_device(cfg),
            #[cfg(feature = "video-decoder")]
            CrossPlatformDevicesCommands::Video(cfg) => run_video_device(cfg),'
      substituteInPlace devices/src/virtio/media.rs \
        --replace-fail 'struct EventQueue(Queue);' 'pub struct EventQueue(pub Queue);' \
        --replace-fail 'struct HostMemoryMapper<M: SharedMemoryMapper> {' 'pub struct HostMemoryMapper<M: SharedMemoryMapper> {' \
        --replace-fail '    shm_mapper: M,' '    pub shm_mapper: M,' \
        --replace-fail '    allocator: AddressAllocator,' '    pub allocator: AddressAllocator,' \
        --replace-fail 'enum Token {' 'pub enum Token {' \
        --replace-fail 'struct WaitContextPoller(Rc<WaitContext<Token>>);' 'pub struct WaitContextPoller(pub Rc<WaitContext<Token>>);'
    '';
  });

  gpuRenderNodeRunner = role: microvm: waylandSocket: gpuParams: {
    binaryPath = "${microvm.graphics.crosvmPackage}/bin/crosvm";
    argv = [
      "d2b-role-${role.roleId}"
      "device" "gpu"
      "--socket" "${roleRuntime role}/gpu.sock"
      "--wayland-sock" waylandSocket
        "--gpu-device-node" "/proc/self/fd/10"
      "--params" gpuParams
    ];
    env = [ "LD_LIBRARY_PATH=${pkgs.vulkan-loader}/lib" ];
  };

  roleNode = workload: microvm: role:
    let
      runtime = roleRuntime role;
      spec = cfg._index.workloads.byId.${workload.workloadId}.spec;
      waylandUid =
        if cfg.site.waylandUser == null
        then 0
        else config.users.users.${cfg.site.waylandUser}.uid;
      wayland = roleFor workload.workloadId "wayland-proxy";
      waylandSocket =
        if wayland == null then null else waylandSocketFor workload.workloadId;
      gpuParams =
        ''{"context-types":"virgl:virgl2:cross-domain","displays":[{"hidden":true}],"egl":true,"vulkan":true}'';
      qemuMemoryMiB =
        lib.attrByPath [ "qemuMedia" "resources" "memoryMiB" ] 2048 spec;
      qemuVcpu =
        lib.attrByPath [ "qemuMedia" "resources" "vcpu" ] 2 spec;
    in
    if role.roleKind == "qemu-media" then mkNode {
      id = role.roleId;
      role = role.processRole;
      ready = [ (socketListening "${runtime}/qmp.sock") ];
      binaryPath =
        if pkgs.stdenv.hostPlatform.system == "x86_64-linux"
        then "${pkgs.qemu_kvm}/bin/qemu-system-x86_64"
        else "${pkgs.qemu_kvm}/bin/qemu-system-aarch64";
      argv = [
        "d2b-qemu-media@${workload.workloadId}"
        "-nodefaults"
        "-no-user-config"
        "-S"
        "-object"
        "memory-backend-ram,id=nlram,size=${toString qemuMemoryMiB}M,dump=off,merge=off"
        "-machine" "q35,accel=kvm,usb=off,memory-backend=nlram"
        "-m" "${toString qemuMemoryMiB}M"
        "-smp" (toString qemuVcpu)
        "-device" "usb-ehci,id=ehci"
        "-device" "virtio-vga"
        "-display" "gtk,gl=off,show-cursor=on"
        "-device" "usb-kbd,bus=ehci.0"
        "-device" "usb-tablet,bus=ehci.0"
        "-netdev" "tap,id=nl0,fd=10,vhost=off"
        "-device"
        "virtio-net-pci,netdev=nl0,mac=${
          if workload.networkInterfaces == [ ]
          then throw "qemu-media workload ${workload.workloadId} has no allocator-declared network interface"
          else (builtins.head workload.networkInterfaces).mac
        }"
        "-qmp" "unix:${runtime}/qmp.sock,server=on,wait=off"
        "-monitor" "none"
        "-chardev" "socket,id=con0,fd=11"
        "-serial" "chardev:con0"
        "-parallel" "none"
        "-name" "workload-${workload.workloadId}"
      ];
      env = lib.optionals (wayland != null) [
        "GDK_BACKEND=wayland"
        "WAYLAND_DISPLAY=wayland-0"
        "XDG_RUNTIME_DIR=${roleRuntime wayland}"
      ];
      networkInterfaces = workload.networkInterfaces;
    }
    else if role.roleKind == "store-virtiofs-preflight" then mkNode {
      id = role.roleId;
      role = role.processRole;
      ready = [
        (commandReady [
          "test" "-e"
          "${workload.storeViewLive}/.d2b-marker-${workload.workloadId}"
        ])
      ];
    }
    else if role.roleKind == "swtpm-pre-start-flush" then mkNode {
      id = role.roleId;
      role = role.processRole;
      binaryPath = toString (swtpmFlushScript workload role);
      argv = [ "d2b-role-${role.roleId}" ];
    }
    else if role.roleKind == "swtpm" then mkNode {
      id = role.roleId;
      role = role.processRole;
      ready = [ (socketListening "${runtime}/tpm.sock") ];
      binaryPath = "${pkgs.swtpm}/bin/swtpm";
      argv = [
        "microvm-swtpm@${workload.workloadId}"
        "socket"
        "--tpmstate" "dir=${workload.stateRoot}/tpm"
        "--ctrl" "type=unixio,path=${runtime}/tpm.sock,mode=0660"
        "--tpm2"
        "--flags" "startup-clear"
      ];
    }
    else if role.roleKind == "gpu"
      && !(microvm.graphics.renderNodeOnly or false) then mkNode {
      id = role.roleId;
      role = role.processRole;
      ready = [ (socketListening "${runtime}/gpu.sock") ];
      binaryPath = "${microvm.graphics.crosvmPackage}/bin/crosvm";
      argv = [
        "d2b-role-${role.roleId}"
        "device" "gpu"
        "--socket" "${runtime}/gpu.sock"
        "--wayland-sock" waylandSocket
         "--gpu-device-node" "/proc/self/fd/10"
        "--params" gpuParams
      ];
      env = [ "LD_LIBRARY_PATH=${pkgs.vulkan-loader}/lib" ];
    }
    else if role.roleKind == "gpu-render-node"
      && (microvm.graphics.renderNodeOnly or false) then
      mkNode ({
        id = role.roleId;
        role = role.processRole;
        ready = [ (socketListening "${runtime}/gpu.sock") ];
      } // gpuRenderNodeRunner role microvm waylandSocket gpuParams)
    else if role.roleKind == "video" then mkNode {
      id = role.roleId;
      role = role.processRole;
      ready = [ (socketListening "${runtime}/video.sock") ];
      binaryPath = "${crosvmVideo}/bin/crosvm";
      argv = [
        "d2b-role-${role.roleId}"
        "device" "video-decoder"
        "--socket-path" "${runtime}/video.sock"
        "--backend" "vaapi"
      ];
      env = [
        "LIBVA_DRM_DEVICE=/proc/self/fd/10"
        "XDG_RUNTIME_DIR=/run/user/${toString waylandUid}"
      ];
    }
    else if role.roleKind == "audio" then
      let audio = audioFor workload.workloadId;
      in mkNode {
        id = role.roleId;
        role = role.processRole;
        ready = [
          (socketListening (builtins.elemAt audio.argv 2))
        ];
        binaryPath = audio.executable.runtimePath;
        inherit (audio) argv;
        env = audio.environment;
      }
    else if role.roleKind == "wayland-proxy" then mkNode {
      id = role.roleId;
      role = role.processRole;
      ready = [ (socketListening waylandSocket) ];
      binaryPath = "${d2bWaylandProxy}/bin/d2b-wayland-proxy";
      argv = [
        "d2b-role-${role.roleId}"
        "--session-generation" "1"
        "--target" workload.canonicalTarget
        "--provider-kind"
        (if workload.runtimeImplementation == "cloud-hypervisor"
         then "local-vm"
         else workload.runtimeImplementation)
        "--realm-id" workload.realmId
        "--workload-id" workload.workloadId
        "--provider-id" (displayMappingFor workload.workloadId).providerId
        "--app-id-prefix" "d2b.${workload.workloadId}."
        "--title-prefix" "[${workload.canonicalTarget}] "
      ];
    }
    else if role.processRole == "otel-host-bridge" then
      let
        endpoints = cfg._realmObservability.endpoints;
      in mkNode {
        id = role.roleId;
        role = role.processRole;
        ready = [ (socketListening endpoints.hostEgress.path) ];
        binaryPath = "${cfg.observability.transport.relayPackage}/bin/socat";
        argv = [
          "d2b-role-${role.roleId}"
          "-d" "-d"
          "UNIX-LISTEN:${endpoints.hostEgress.path},fork,reuseaddr,mode=0660"
          ''EXEC:"${d2bChVsockConnect}/bin/d2b-ch-vsock-connect ${endpoints.stackVsock.path} ${toString endpoints.stackVsock.port}"''
        ];
      }
    else if role.roleKind == "vsock-relay" then mkNode {
      id = role.roleId;
      role = role.processRole;
      ready = [ (componentReady "realm-controller guest transport relay") ];
    }
    else if role.roleKind == "guest-control-health" then mkNode {
      id = role.roleId;
      role = role.processRole;
      ready = [
        (readiness "guest-control-health" {
          vm = workload.workloadId;
        })
      ];
    }
    else if role.roleKind == "security-key-frontend" then mkNode {
      id = role.roleId;
      role = role.processRole;
      ready = [ (componentReady "allocator-owned security-key endpoint") ];
    }
    else if role.roleKind == "usbip" then mkNode {
      id = role.roleId;
      role = role.processRole;
      ready = [ (componentReady "allocator-owned USBIP attachment") ];
    }
    else if role.roleKind == "cloud-hypervisor" then mkNode {
      id = role.roleId;
      role = role.processRole;
      ready = [ (readiness "api-socket-info" "${runtime}/api.sock") ];
      binaryPath = "${microvm.cloud-hypervisor.package}/bin/cloud-hypervisor";
      argv = cloudHypervisorArgv workload microvm;
      env = [ "PATH=${pkgs.coreutils}/bin:${pkgs.gnused}/bin" ];
      networkInterfaces = workload.networkInterfaces;
    }
    else null;

  workloadDag = workload:
    let
      isCloud = workload.runtimeImplementation == "cloud-hypervisor";
      computed = cfg._computedWorkloads.${workload.workloadId} or null;
      microvm = if computed == null then null else computed.config.microvm;
      workloadRoles = lib.filter
        (row: row.workloadId == workload.workloadId)
        roles;
      selectedRoles = lib.filter
        (role:
          role.roleKind != "virtiofsd"
          && !(isCloud && role.roleKind == "gpu"
            && (microvm.graphics.renderNodeOnly or false))
          && !(isCloud && role.roleKind == "gpu-render-node"
            && !(microvm.graphics.renderNodeOnly or false)))
        workloadRoles;
      normalNodes =
        if isCloud && microvm == null
        then [ ]
        else lib.filter (node: node != null)
          (map (roleNode workload microvm) selectedRoles);
      shareNodes =
        if !isCloud || microvm == null
        then [ ]
        else virtiofsNodes workload microvm;
      nodes = normalNodes ++ shareNodes;
      preflight = roleFor workload.workloadId "store-virtiofs-preflight";
      hypervisor = roleFor workload.workloadId "cloud-hypervisor";
      qemu = roleFor workload.workloadId "qemu-media";
      wayland = roleFor workload.workloadId "wayland-proxy";
      dependencyNodes = map (node: node.id)
        (lib.filter (node:
          node.role != "cloud-hypervisor-runner"
          && node.role != "guest-control-health")
          nodes);
      dagEdges =
        lib.optionals (preflight != null)
          (map
            (node:
              edge preflight.roleId node.id
                "The store/resource preflight precedes role startup.")
            (lib.filter
              (node:
                node.id != preflight.roleId
                && node.role != "cloud-hypervisor-runner"
                && node.role != "guest-control-health")
              nodes))
        ++ lib.optionals (hypervisor != null)
          (map
            (id:
              edge id hypervisor.roleId
                "Every sidecar is ready before the workload runner starts.")
            (lib.filter (id: id != preflight.roleId) dependencyNodes))
        ++ lib.optionals (hypervisor != null)
          (map
            (node:
              edge hypervisor.roleId node.id
                "Guest protocol readiness follows workload runner startup.")
            (lib.filter
              (node: node.role == "guest-control-health")
              nodes))
        ++ lib.optionals (qemu != null && wayland != null) [
          (edge wayland.roleId qemu.roleId
            "The mediated display endpoint is ready before QEMU starts.")
        ];
    in
    {
      vm = workload.workloadId;
      workloadIdentity = {
        inherit (workload)
          workloadId
          workloadName
          realmId
          canonicalTarget
          ;
        realmPath = lib.splitString "." workload.realmPath;
        runtimeKind = workload.runtimeImplementation;
        providerId = workload.runtimeBinding.providerId;
      };
      inherit nodes;
      edges = dagEdges;
      invariants = {
        perVmAuditPipeline = true;
        swtpmPreStartFlush = true;
        tpmOwnershipMigrationWithoutRunningVmMutation = true;
        usbipGating = true;
      };
    };

  dags = map workloadDag workloads;
  processesData = {
    schemaVersion = "v2";
    vms = dags;
  };
  processesFile = pkgs.writeText "d2b-processes-v2.json"
    (builtins.toJSON processesData);
in
{
  config.d2b = {
    _hostToolPackages.d2bWaylandProxy = d2bWaylandProxy;
    _bundle.processesJson = {
      data = processesData;
      jsonText = builtins.toJSON processesData;
      path = processesFile;
      installFileName = "processes.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };
}
