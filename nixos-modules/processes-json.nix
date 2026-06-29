{ config, lib, pkgs, ... }:

let
  clean = builtins.unsafeDiscardStringContext;

  cfg = config.d2b;
  prebuilt =
    if cfg.site.usePrebuiltHostTools
    then import ./prebuilt-packages.nix { inherit pkgs lib; }
    else { };
  # d2b-owned access helpers (see lib.nix).
  d2bLib = import ./lib.nix { inherit lib pkgs; };
  normalNixosVms = d2bLib.normalNixosVms cfg.vms;
  qemuMediaVms = d2bLib.qemuMediaVms cfg.vms;
  obsOtlpPort = cfg._index.observability.sourceBasePort;
  obsSourcePort = name: cfg._index.observability.sourcePorts.${name} or obsOtlpPort;
  waylandUid =
    if cfg.site.waylandUser != null
    then toString (config.users.users.${cfg.site.waylandUser}.uid or 0)
    else "0";
  waylandDisplay = cfg.site.waylandDisplay;
  # Real host compositor socket. Used only by the wayland-proxy role;
  # GPU runners no longer reference this path directly.
  waylandHostSock = "/run/user/${waylandUid}/${waylandDisplay}";
  chVsockConnect = import ./d2b-ch-vsock-connect.nix { inherit pkgs; };
  vhostDeviceSound = import ../pkgs/vhost-device-sound { inherit pkgs; };
  spectrumCH = import ../pkgs/spectrum-ch { inherit pkgs; };

  # d2b-wayland-proxy: host-side Wayland proxy.
  # Built from the workspace so the binary path is available for the
  # wayland-proxy DAG node's binaryPath field.
  packagesSrc = d2bLib.cleanRustPackagesSource ../packages;
  d2bWaylandProxySourcePackage = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b-wayland-proxy";
    version = "0.0.0";
    src = packagesSrc;
    cargoLock = {
      lockFile = ../packages/Cargo.lock;
      outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
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
      install -Dm755 target/x86_64-unknown-linux-gnu/release/d2b-wayland-proxy $out/bin/d2b-wayland-proxy 2>/dev/null \
        || install -Dm755 target/release/d2b-wayland-proxy $out/bin/d2b-wayland-proxy
      runHook postInstall
    '';
  };
  # The filter is tied to the checked-out policy implementation and is cheap
  # enough to build in the eval smoke fixtures. Keep it source-built even when
  # other host tools use release prebuilts so missing release assets cannot
  # break local validation.
  d2bWaylandProxyPackage = d2bWaylandProxySourcePackage;
  d2bWaylandProxyBinary = "${d2bWaylandProxyPackage}/bin/d2b-wayland-proxy";

  backendPort = envName: cfg._index.usbip.backendPorts.${envName};

  profileIdFor = name: nodeId: "vm-${name}-${nodeId}";
  profileFor = name: nodeId:
    if nodeId == "otel-host-bridge"
    then cfg._bundle.minijailProfiles."host-otel-host-bridge".roleProfile
    else cfg._bundle.minijailProfiles.${profileIdFor name nodeId}.roleProfile;
  vsockSocketForPort = socketPath: port: "${socketPath}_${toString port}";
  shareNodeId = share: "virtiofsd-${clean share.tag}";
  shareSocketPath = name: share:
    if share.tag == "d2b-gctl"
    then "/run/d2b/vms/${name}/guest-control/${clean share.tag}.sock"
    else "/run/d2b/vms/${name}/${clean share.tag}.sock";
  volumeHostPath = name: volume: d2bLib.volumeHostPath cfg.store.stateDir name volume;

  mkReadiness = kind: value: { inherit kind value; };
  componentReady = mkReadiness "component-specific";
  apiSocketInfo = mkReadiness "api-socket-info";
  unixSocketExists = mkReadiness "unix-socket-exists";
  unixSocketListening = mkReadiness "unix-socket-listening";
  tcpPort = host: port: mkReadiness "tcp-port" { inherit host port; };
  commandReady = mkReadiness "command";
  # Authenticated guest-control Health readiness. Unlike a raw TCP-22
  # probe this predicate fails CLOSED: the daemon completes a full
  # Hello + token challenge-response + Health over the guest-control vsock
  # before the node is ready. The daemon resolves the per-VM vsock socket,
  # peer credentials, and broker-backed signer from its own trusted state.
  guestControlHealthReady = vmName: { kind = "guest-control-health"; value = { vm = vmName; }; };

  extractOptValues = optFlag: extraArgs:
    let
      flags = if builtins.isList optFlag then optFlag else [ optFlag ];
      processArgs = args: values: acc:
        if args == [ ] then
          { inherit values; args = acc; }
        else if (builtins.elem (builtins.head args) flags) && (builtins.length args) > 1 then
          processArgs (builtins.tail (builtins.tail args)) (values ++ [ (builtins.elemAt args 1) ]) acc
        else
          processArgs (builtins.tail args) values (acc ++ [ (builtins.head args) ]);
    in
    processArgs extraArgs [ ] [ ];

  extractParamValue = param: opts:
    if opts == "" || opts == null then null
    else
      let
        m = builtins.match ".*${param}=([^,]+).*" opts;
      in
      if m == null then null else builtins.head m;

  opsMapped = ops:
    lib.concatStringsSep "," (
      lib.mapAttrsToList (k: v: "${k}=${toString v}") ops
    );

  # CH 52 changed `--fs`/`--net`/`--disk`/`--device` from
  # accepting repeated `--flag value` pairs to a single `--flag` followed
  # by multiple positional values (clap variadic). The microvm.nix-derived
  # net-VM argv path used the OLD repeated-flag style which CH 52 rejects
  # with "the argument '--fs <fs>...' cannot be used multiple times".
  #
  # Fix: emit single-flag variadic for the `--fs`/`--net`/`--disk`/
  # `--device` cases. Other CH variadic flags (`--vsock`, `--gpu`) only
  # ever have one value and are not affected.
  variadicFlagArgs = flag: params:
    if params == [ ]
    then [ ]
    else [ flag ] ++ params;

  resolvedInterfaces = microvm:
    builtins.map (iface: {
      type = iface.type;
      id = iface.id or null;
      mac = iface.mac or null;
    }) microvm.interfaces;

  resolvedVirtiofsdThreadPoolSize = microvm:
    let
      raw = microvm.virtiofsd.threadPoolSize;
    in
    if builtins.isInt raw then
      toString raw
    else if builtins.typeOf raw == "string" && builtins.match "^[0-9]+$" raw != null then
      raw
    else
      toString (if microvm.vcpu > 0 then microvm.vcpu else 1);

  swtpmFlushScript = name:
    pkgs.writeShellScript "d2b-${name}-swtpm-flush" ''
      set -eu
      state_dir=/var/lib/d2b/vms/${name}/swtpm
      permall_file="$state_dir/swtpm_perm.state"
      flush_sock=/run/d2b/vms/${name}/tpm-flush.sock

      if [ ! -f "$permall_file" ]; then
        exit 0
      fi

      cleanup() {
        rm -f "$flush_sock"
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

      for i in $(${pkgs.coreutils}/bin/seq 1 50); do
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

  cloudHypervisorBinaryPath = microvm: "${microvm.cloud-hypervisor.package}/bin/cloud-hypervisor";
  qemuMediaQmpSocket = name: "/run/d2b/vms/${name}/qmp.sock";
  qemuMediaBinaryPath =
    if pkgs.stdenv.hostPlatform.system == "x86_64-linux" then
      "${pkgs.qemu_kvm}/bin/qemu-system-x86_64"
    else if pkgs.stdenv.hostPlatform.system == "aarch64-linux" then
      "${pkgs.qemu_kvm}/bin/qemu-system-aarch64"
    else
      throw "Unsupported system ${pkgs.stdenv.hostPlatform.system} for qemu-media argv emission";

  qemuMediaMac = name:
    let vm = cfg.vms.${name};
    in d2bLib.mkMac vm.env "lan" vm.index;
  qemuMediaArgv = name:
    let
      vm = cfg.vms.${name};
      resources = vm.qemuMedia.resources;
      security = vm.qemuMedia.security;
      memoryBackendFlags = [
        "memory-backend-ram"
        "id=nlram"
        "size=${toString resources.memoryMiB}M"
        "dump=${if security.excludeMemoryFromCoreDump then "off" else "on"}"
        "merge=${if security.disableMemoryMerge then "off" else "on"}"
      ] ++ lib.optional security.lockMemory "prealloc=on";
    in [
      "d2b-qemu-media@${name}"
      "-nodefaults"
      "-no-user-config"
      "-S"
      "-object"
      (lib.concatStringsSep "," memoryBackendFlags)
      "-machine"
      "q35,accel=kvm,usb=off,memory-backend=nlram"
      "-m"
      "${toString resources.memoryMiB}M"
      "-smp"
      (toString resources.vcpu)
    ] ++ lib.optionals security.lockMemory [
      "-overcommit"
      "mem-lock=on"
    ] ++ [
      "-device"
      "usb-ehci,id=ehci"
      "-device"
      "virtio-vga"
      "-display"
      "gtk,gl=off,show-cursor=on"
      "-device"
      "usb-kbd,bus=ehci.0"
      "-device"
      "usb-tablet,bus=ehci.0"
      "-netdev"
      "tap,id=nl0,fd=10,vhost=off"
      "-device"
      "virtio-net-pci,netdev=nl0,mac=${qemuMediaMac name}"
      "-qmp"
      "unix:${qemuMediaQmpSocket name},server=on,wait=off"
      "-monitor"
      "none"
      "-chardev"
      "socket,id=con0,fd=11"
      "-serial"
      "chardev:con0"
      "-parallel"
      "none"
      "-name"
      "d2b-${name}-qemu-media"
    ];
  qemuMediaEnv = name:
    lib.optionals (cfg.site.waylandUser != null) [
      "GDK_BACKEND=wayland"
      "WAYLAND_DISPLAY=wayland-0"
      "XDG_RUNTIME_DIR=/run/d2b-wlproxy/${name}"
    ];

  cloudHypervisorArgv = name: vm: manifest:
    let
      microvm = d2bLib.vmRunner config name;
      extraArgs = microvm.cloud-hypervisor.extraArgs;
      hasUserVsockExtraArg = lib.any
        (arg: arg == "--vsock" || lib.hasPrefix "--vsock=" arg)
        extraArgs;
      processedExtraArgs = builtins.foldl'
        (args: opt: (extractOptValues opt args).args)
        extraArgs
        [ "--platform" ];
      hasUserConsole = (extractOptValues "--console" extraArgs).values != [ ];
      userSerialValues = (extractOptValues "--serial" extraArgs).values;
      hasUserSerial = userSerialValues != [ ];
      userSerial = if hasUserSerial then builtins.head userSerialValues else null;
      kernelPath =
        if pkgs.stdenv.hostPlatform.system == "x86_64-linux" then
          "${microvm.kernel.dev}/vmlinux"
        else if pkgs.stdenv.hostPlatform.system == "aarch64-linux" then
          "${microvm.kernel.out}/${pkgs.stdenv.hostPlatform.linux-kernel.target}"
        else
          throw "Unsupported system ${pkgs.stdenv.hostPlatform.system} for cloud-hypervisor argv emission";
      kernelConsoleDefault =
        if pkgs.stdenv.hostPlatform.system == "x86_64-linux" then
          "earlyprintk=ttyS0 console=ttyS0"
        else if pkgs.stdenv.hostPlatform.system == "aarch64-linux" then
          "console=ttyAMA0"
        else
          "";
      kernelConsole = if (!hasUserSerial) || userSerial == "tty" then kernelConsoleDefault else "";
      kernelCmdLine = lib.concatStringsSep " " (
        lib.filter (value: value != "") ([ kernelConsole "reboot=t" "panic=-1" ] ++ microvm.kernelParams)
      );
      vsockCID = microvm.vsock.cid;
      vsockPath = microvm.vsock.socket;
      supportsNotifySocket = true;
      vsockOpts =
        if hasUserVsockExtraArg then
          throw "d2b.vms.${name}.config.microvm.cloud-hypervisor.extraArgs must not set --vsock; d2b owns the Cloud Hypervisor vsock device for guest control and observability"
        else
          "cid=${toString vsockCID},socket=${vsockPath}";
      virtiofsShares = lib.filter (share: (share.proto or "virtiofs") == "virtiofs") microvm.shares;
      useVirtiofs = virtiofsShares != [ ];
      useHotPlugMemory = microvm.hotplugMem > 0;
      memOps = opsMapped ({
        size = "${toString microvm.mem}M";
        shared = if useVirtiofs || vm.graphics.enable then "on" else "off";
      }
      // lib.optionalAttrs (!useVirtiofs && !vm.graphics.enable) {
        mergeable = "on";
      }
      // lib.optionalAttrs useHotPlugMemory {
        size = "${toString microvm.hotplugMem}M";
        hotplug_method = "virtio-mem";
        hotplug_size = "${toString microvm.hotplugMem}M";
        hotplugged_size = "${toString microvm.hotpluggedMem}M";
      }
      // lib.optionalAttrs microvm.hugepageMem {
        hugepages = "on";
      });
      balloonOps = opsMapped ({
        size = "${toString microvm.initialBalloonMem}M";
        free_page_reporting = "on";
      }
      // lib.optionalAttrs microvm.deflateOnOOM {
        deflate_on_oom = "on";
      });
      tapMultiQueue = microvm.vcpu > 1;
      diskMqOps = lib.optionalAttrs tapMultiQueue {
        num_queues = toString microvm.vcpu;
      };
      netMqOps = lib.optionalAttrs tapMultiQueue {
        num_queues = toString (2 * microvm.vcpu);
      };
      oemStringValues = microvm.cloud-hypervisor.platformOEMStrings ++ lib.optional supportsNotifySocket "io.systemd.credential:vmm.notify_socket=vsock-stream:2:8888";
      oemStringOptions = lib.optional (oemStringValues != [ ]) "oem_strings=[${lib.concatStringsSep "," oemStringValues}]";
      platformExtracted = extractOptValues "--platform" extraArgs;
      userPlatformOpts = platformExtracted.values;
      userPlatformStr = if userPlatformOpts != [ ] then builtins.head userPlatformOpts else "";
      userHasOemStrings = (extractParamValue "oem_strings" userPlatformStr) != null;
      platformOps =
        if userHasOemStrings then
          throw "Use microvm.cloud-hypervisor.platformOEMStrings instead of passing oem_strings via --platform"
        else
          lib.concatStringsSep "," (oemStringOptions ++ userPlatformOpts);
      fsParams = builtins.map (share:
        opsMapped {
          tag = share.tag;
          socket = shareSocketPath name share;
        }
      ) virtiofsShares;
      diskParams =
        lib.optional microvm.storeOnDisk (opsMapped ({
          path = toString microvm.storeDisk;
          readonly = "on";
        } // diskMqOps))
        ++ builtins.map (volume:
          opsMapped ({
            # prefix relative volume.image with the
            # per-VM state dir so CH (which has no cwd under broker
            # spawn) can find the disk. Absolute paths pass through
            # unchanged.
            path = volumeHostPath name volume;
            # Defensive defaults for volume fields the consumer may
            # omit. The d2b-owned vm-options.nix
            # types `microvm.volumes` as `listOf attrs` (untyped)
            # for forward compat; processes-json.nix supplies the
            # CH defaults when the consumer doesn't.
            direct = if (volume.direct or false) then "on" else "off";
            readonly = if (volume.readOnly or false) then "on" else "off";
            image_type = toString (volume.imageType or "raw");
            serial = d2bLib.volumeSerial volume;
          }
          // diskMqOps)
        ) microvm.volumes
        ++ lib.optionals (microvm.writableStoreOverlay != null) [
          (opsMapped ({
            # writableStoreOverlay is a guest-side overlayfs upper
            # layer. The backing image is provisioned by the broker's
            # `DiskInit` plan-op at host start; the broker also runs
            # mkfs.ext4 on the new image so the guest kernel can
            # mount it.  CRITICAL: this disk MUST include the same
            # CH disk argv defaults (`direct`, `image_type`,
            # `num_queues`) the regular volume path emits — without
            # them CH 52 falls back to an auto-detected mode that
            # leaves the guest unable to bring up the
            # /nix/store-overlayfs upper, hanging early in
            # initramfs before earlyprintk produces output.
            path = "${toString cfg.store.stateDir}/${name}/store-overlay.img";
            serial = "rootfs";
            direct = "off";
            readonly = "off";
            image_type = "raw";
          } // diskMqOps))
        ];
      netParams = builtins.map (iface:
        if iface.type == "tap" then
          opsMapped ({
            tap = iface.id;
            mac = iface.mac;
          } // netMqOps)
        else
          throw "Unsupported interface type ${iface.type} for cloud-hypervisor argv emission"
      ) (resolvedInterfaces microvm);
      deviceParams = builtins.map (device:
        # Defensive default for device.bus when the consumer doesn't
        # specify (defaults to "pci" for the
        # contract preserved here).
        let bus = device.bus or "pci"; in
        if bus == "pci" then
          "path=/sys/bus/pci/devices/${device.path}"
        else
          throw "Unsupported device bus ${bus} for cloud-hypervisor argv emission"
      ) microvm.devices;
      audioExtraArgs = lib.optionals vm.audio.enable [
        "--generic-vhost-user"
        "socket=/run/d2b/vms/${name}/snd.sock,virtio_id=25,queue_sizes=[64,64,64,64]"
      ];
    in
    [
      "microvm@${name}"
      "--cpus"
      "boot=${toString microvm.vcpu}"
      "--watchdog"
      "--kernel"
      kernelPath
      "--initramfs"
      (toString microvm.initrdPath)
      "--cmdline"
      kernelCmdLine
      "--seccomp"
      "true"
      "--memory"
      memOps
      "--platform"
      platformOps
    ]
    ++ lib.optionals (!hasUserConsole) [ "--console" "null" ]
    ++ lib.optionals (!hasUserSerial) [ "--serial" "tty" ]
    ++ [ "--vsock" vsockOpts ]
    ++ lib.optionals vm.graphics.enable [ "--gpu" "socket=${microvm.graphics.socket}" ]
    ++ lib.optionals microvm.balloon [ "--balloon" balloonOps ]
    ++ variadicFlagArgs "--disk" diskParams
    ++ variadicFlagArgs "--fs" fsParams
    ++ [ "--api-socket" manifest.apiSocket ]
    ++ variadicFlagArgs "--net" netParams
    ++ variadicFlagArgs "--device" deviceParams
    ++ processedExtraArgs
    ++ audioExtraArgs;

  mediaArgValues = name: vm: manifest:
    (extractOptValues "--vhost-user-media" (cloudHypervisorArgv name vm manifest)).values;
  mediaFlagTokens = name: vm: manifest:
    builtins.filter
      (arg: builtins.isString arg && lib.hasPrefix "--vhost-user-media" arg)
      (cloudHypervisorArgv name vm manifest);

  virtiofsdRunner = name: share:
    let
      microvm = d2bLib.vmRunner config name;
      # (ADR 0021): under broker-pre-NS, virtiofsd is
      # fake-root inside its own user namespace. Use --sandbox=chroot
      # (now works because we have CAP_SYS_ADMIN inside the NS) and
      # disable file handles (we don't need open_by_handle_at(2)
      # for read-only or per-VM share serving). --posix-acl + --xattr
      # are dropped: /nix/store has no ACLs, and the per-VM shares
      # are d2b-managed (no foreign xattrs to preserve).
      isRoStore = share.source == "/nix/store";
      isStoreMeta = share.tag == "d2b-meta";
      # SECURITY (per-VM store isolation): the ro-store share's guest
      # `/nix/store` must expose ONLY this VM's closure, never the host's
      # full `/nix/store`. `share.source` stays `/nix/store` as the
      # eval-time sentinel that the guest-mount + overlay + readiness
      # logic keys off, but virtiofsd is pointed at the per-VM hardlink
      # live pool `<stateDir>/<vm>/store-view/live` — the canonical
      # closure-only per-VM store. virtiofsd still execs from the real host
      # `/nix/store` (kept mounted in its runner namespace) and only
      # *serves* the farm, so the guest sees a closure-only store. This
      # replaces the previous `--shared-dir=/nix/store`, which leaked the
      # host's entire store into every guest. Mirrors the legacy
      # `BindReadOnlyPaths /nix/store -> per-VM farm` behaviour.
      roStoreSharedDir = "${toString cfg.store.stateDir}/${name}/store-view/live";
      sharedDir = if isRoStore then roStoreSharedDir else toString share.source;
    in {
      binaryPath = "${microvm.virtiofsd.package}/bin/virtiofsd";
      argv = [
        "microvm-virtiofsd@${name}-${clean share.tag}"
        "--socket-path=${shareSocketPath name share}"
      ]
      ++ lib.optionals (microvm.virtiofsd.group != null) [ "--socket-group=${microvm.virtiofsd.group}" ]
      ++ [
        "--shared-dir=${sharedDir}"
        "--thread-pool-size"
        (resolvedVirtiofsdThreadPoolSize microvm)
        "--sandbox=chroot"
        "--inode-file-handles=never"
        "--cache=${share.cache or "auto"}"
      ]
      ++ lib.optionals (microvm.hypervisor == "crosvm") [ "--tag=${share.tag}" ]
      ++ lib.optionals (isRoStore || isStoreMeta || (share.readOnly or false)) [ "--readonly" ]
      ++ microvm.virtiofsd.extraArgs;
    };

  swtpmFlushRunner = name:
    let
      script = swtpmFlushScript name;
    in {
      binaryPath = "${script}";
      argv = [ "d2b-swtpm-flush@${name}" ];
    };

  swtpmRunner = name: {
    binaryPath = "${pkgs.swtpm}/bin/swtpm";
    argv = [
      "microvm-swtpm@${name}"
      "socket"
      "--tpmstate"
      "dir=/var/lib/d2b/vms/${name}/swtpm"
      "--ctrl"
      # mode=0660 (was 0600) so that combined with
      # umask 0o007 from the swtpm role profile and the per-VM
      # /run/d2b/vms/<vm>/ default ACL granting CH's UID rw,
      # cloud-hypervisor can connect to the TPM control socket
      # without operator setfacl intervention.
      "type=unixio,path=/run/d2b/vms/${name}/tpm.sock,mode=0660"
      "--tpm2"
      "--flags"
      "startup-clear"
    ];
  };

  gpuRunner = name: vm:
    let
      microvm = d2bLib.vmRunner config name;
      gpuParams = "{\"context-types\":\"virgl:virgl2:cross-domain\",\"displays\":[{\"hidden\":true}],\"egl\":true,\"vulkan\":true}";
      filterSock = "/run/d2b-wlproxy/${name}/wayland-0";
      emitWaylandProxy = vm.graphics.enable && vm.graphics.crossDomainTrusted && vm.graphics.waylandProxy.enable;
      # When the Wayland proxy is emitted, crosvm connects to the proxy
      # socket. Otherwise preserve the legacy display backend by connecting
      # directly to the real host compositor socket.
      waylandSock = if emitWaylandProxy then filterSock else waylandHostSock;
    in {
      binaryPath = "${microvm.graphics.crosvmPackage}/bin/crosvm";
      argv = [
        "d2b-${name}-gpu"
        "device"
        "gpu"
        "--socket"
        microvm.graphics.socket
        "--wayland-sock"
        waylandSock
        "--params"
        gpuParams
      ];
      env = [
        "LD_LIBRARY_PATH=${pkgs.vulkan-loader}/lib"
      ];
    };

  # (ADR 0021) render-node-only broker-pre-NS GPU sidecar.
  #
  # This runner is identical to gpuRunner except
  #   --gpu-device-node /proc/self/fd/10 : references the pre-opened render
  #     node fd that the broker dup2'd to RENDER_NODE_INHERITED_FD (10) in
  #     the user-NS child before execve.
  #
  # The broker parent opens /dev/dri/renderD128, dup2's it to fd 10 in the
  # child, and the crosvm process accesses it via /proc/self/fd/10 without
  # ever needing host-side DAC access to /dev/dri/.
  gpuRenderNodeRunner = name: vm:
    let
      microvm = d2bLib.vmRunner config name;
      gpuParams = "{\"context-types\":\"virgl:virgl2:cross-domain\",\"displays\":[{\"hidden\":true}],\"egl\":true,\"vulkan\":true}";
      filterSock = "/run/d2b-wlproxy/${name}/wayland-0";
      emitWaylandProxy = vm.graphics.enable && vm.graphics.crossDomainTrusted && vm.graphics.waylandProxy.enable;
      waylandSock = if emitWaylandProxy then filterSock else waylandHostSock;
    in {
      binaryPath = "${microvm.graphics.crosvmPackage}/bin/crosvm";
      argv = [
        "d2b-${name}-gpu-render-node"
        "device"
        "gpu"
        "--socket"
        microvm.graphics.socket
        "--wayland-sock"
        waylandSock
        # reference the pre-opened render node fd.
        # The broker dup2'd /dev/dri/renderD128 to fd 10
        # (RENDER_NODE_INHERITED_FD) in the user-NS child before execve.
        "--gpu-device-node"
        "/proc/self/fd/10"
        "--params"
        gpuParams
      ];
      env = [
        "LD_LIBRARY_PATH=${pkgs.vulkan-loader}/lib"
      ];
    };

  # wayland-proxy runner: d2b-wayland-proxy host-side proxy.
  # Runs as d2b-<vm>-wlproxy, listens on the per-VM proxy socket,
  # and connects upstream to the real host compositor socket. The broker
  # grants the wlproxy principal an ACL on exactly that socket.
  waylandProxyRunner = name: vm:
    let
      vmName = name;
      filterSock = "/run/d2b-wlproxy/${vmName}/wayland-0";
      upstreamSock = waylandHostSock;
      bridgeSock = "${config.d2b.site.clipboard.runtime.bridgeRoot}/${waylandUid}/bridge/${vmName}/${config.d2b.site.clipboard.runtime.bridgeSocketName}";
      appIdPrefix = "d2b.${vmName}.";
      titlePrefix = "[${vmName}] ";
      denyArgs = lib.concatMap (g: [ "--deny-global" g ]) vm.graphics.waylandProxy.denyGlobals;
      allowArgs = lib.concatMap (g: [ "--allow-global" g ]) vm.graphics.waylandProxy.allowGlobals;
      maxVersionArgs = lib.concatMap
        (nameVersion:
          let parts = lib.splitString "=" nameVersion;
          in [ "--max-version" nameVersion ])
        (lib.mapAttrsToList (iface: ver: "${iface}=${toString ver}") vm.graphics.waylandProxy.maxVersions);
      dmabufAllowArgs = lib.concatMap (filter: [ "--dmabuf-allow" filter ]) vm.graphics.waylandProxy.dmabufAllow;
      dmabufDenyArgs = lib.concatMap (filter: [ "--dmabuf-deny" filter ]) vm.graphics.waylandProxy.dmabufDeny;
    in {
      binaryPath = d2bWaylandProxyBinary;
      env = [
        "XDG_RUNTIME_DIR=/run/user/${waylandUid}"
        "WAYLAND_DISPLAY=${waylandDisplay}"
      ] ++ lib.optionals vm.graphics.waylandProxy.debugLogging [
        "WL_PROXY_DEBUG=1"
        "WL_PROXY_PREFIX=d2b-${vmName}-wlproxy"
      ] ++ lib.optionals vm.graphics.waylandProxy.byteLogging [
        "WL_PROXY_HEXDUMP=1"
        "WL_PROXY_HEXDUMP_LIMIT=256"
      ];
      argv = [
        "d2b-${vmName}-wlproxy"
        "--listen" filterSock
        "--connect" upstreamSock
        "--vm-name" vmName
        "--app-id-prefix" appIdPrefix
        "--title-prefix" titlePrefix
      ] ++ lib.optionals config.d2b.site.clipboard.enable [
        "--clipd-bridge-socket" bridgeSock
      ] ++ denyArgs ++ allowArgs ++ maxVersionArgs ++ dmabufAllowArgs ++ dmabufDenyArgs;
    };

  videoBinaryPath = _name:
    # the per-VM
    # `d2b-${name}-video.service` was deleted. The video
    # sidecar is now broker-spawned via SpawnRunner{role: Video},
    # and the broker takes the binary path from the bundle's
    # helperPaths map (populated by this file's emitter below).
    # We construct the crosvmVideo binary path inline using the
    # same overlay the deleted systemd template used.
    "${crosvmVideo}/bin/crosvm";

  # crosvmVideo derivation
  # relocated here from the deleted
  # nixos-modules/components/video/host.nix. The overlay adds the
  # video-decoder + vaapi + media features to crosvm and patches
  # in the vhost-user video backend from
  # pkgs/vhost-user-video/. The binary path is consumed by the
  # broker via the bundle's helperPaths map.
  crosvmVideo = (pkgs.crosvm.overrideAttrs (old: {
    buildInputs = (old.buildInputs or []) ++ [ pkgs.libva ];
    cargoBuildFeatures = (old.cargoBuildFeatures or old.buildFeatures or []) ++ [
      "video-decoder" "vaapi" "media"
    ];
    cargoCheckFeatures = (old .cargoCheckFeatures or old.cargoBuildFeatures or old.buildFeatures or []) ++ [
      "video-decoder" "vaapi" "media"
    ];
    postPatch = (old.postPatch or "") + ''
      mkdir -p devices/src/virtio/vhost_user_backend/video/sys
      cp ${../pkgs/vhost-user-video/mod.rs} devices/src/virtio/vhost_user_backend/video/mod.rs
      cp ${../pkgs/vhost-user-video/sys_mod.rs} devices/src/virtio/vhost_user_backend/video/sys/mod.rs
      cp ${../pkgs/vhost-user-video/sys_linux.rs} devices/src/virtio/vhost_user_backend/video/sys/linux.rs

      substituteInPlace devices/src/virtio/vhost_user_backend/mod.rs \
        --replace-fail \
          '#[cfg(feature = "audio")]
pub mod snd;' \
          '#[cfg(feature = "audio")]
pub mod snd;
#[cfg(feature = "video-decoder")]
pub mod video;'

      substituteInPlace devices/src/virtio/vhost_user_backend/mod.rs \
        --replace-fail \
          '#[cfg(feature = "audio")]
pub use snd::run_snd_device;
#[cfg(feature = "audio")]
pub use snd::Options as SndOptions;' \
          '#[cfg(feature = "audio")]
pub use snd::run_snd_device;
#[cfg(feature = "audio")]
pub use snd::Options as SndOptions;
#[cfg(feature = "video-decoder")]
pub use video::run_video_device;
#[cfg(feature = "video-decoder")]
pub use video::Options as VideoOptions;'

      substituteInPlace src/crosvm/cmdline.rs \
        --replace-fail \
          '#[cfg(feature = "audio")]
    Snd(vhost_user_backend::SndOptions),' \
          '#[cfg(feature = "audio")]
    Snd(vhost_user_backend::SndOptions),
    #[cfg(feature = "video-decoder")]
    Video(vhost_user_backend::VideoOptions),'

      substituteInPlace src/main.rs \
        --replace-fail \
          '#[cfg(feature = "audio")]
use devices::virtio::vhost_user_backend::run_snd_device;' \
          '#[cfg(feature = "audio")]
use devices::virtio::vhost_user_backend::run_snd_device;
#[cfg(feature = "video-decoder")]
use devices::virtio::vhost_user_backend::run_video_device;'

      substituteInPlace src/main.rs \
        --replace-fail \
          '#[cfg(feature = "audio")]
            CrossPlatformDevicesCommands::Snd(cfg) => run_snd_device(cfg),' \
          '#[cfg(feature = "audio")]
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
  }));

  videoRunner = name: {
    binaryPath = videoBinaryPath name;
    argv = [
      "d2b-${name}-video"
      "device"
      "video-decoder"
      "--socket-path"
      "/run/d2b-video/${name}/video.sock"
      "--backend"
      "vaapi"
    ];
    # (Option B): video sidecar uses vaapi which
    # talks to /dev/dri/renderD128 directly, but needs
    # XDG_RUNTIME_DIR for libdrm + intel-media-driver state.
    env = [
      "XDG_RUNTIME_DIR=/run/user/${waylandUid}"
    ];
  };

  audioRunner = name: {
    binaryPath = "${vhostDeviceSound}/bin/vhost-device-sound";
    argv = [
      "d2b-${name}-snd"
      "--socket"
      "/run/d2b/vms/${name}/snd.sock"
      "--backend"
      "pipewire"
    ];
    # (Option B): point libpipewire at the Wayland
    # user's PipeWire socket. Without these env vars,
    # vhost-device-sound (running as the ephemeral role UID)
    # looks at /run/user/$EUID/pipewire-0 which doesn't exist
    # for the ephemeral UID. The PipeWire socket itself is
    # grant'd to the ephemeral UID by host-activation.nix's
    # d2bRoleUidAcls script.
    env = [
      "PIPEWIRE_RUNTIME_DIR=/run/user/${waylandUid}"
      "XDG_RUNTIME_DIR=/run/user/${waylandUid}"
      ''PIPEWIRE_PROPS={ application.name = "d2b-${name}" node.name = "d2b-${name}" node.description = "d2b ${name}" d2b.vm = "${name}" }''
      "WPCTL_PATH=${pkgs.wireplumber}/bin/wpctl"
      "PW_DUMP_PATH=${pkgs.pipewire}/bin/pw-dump"
    ] ++ lib.optional (cfg.site.audio.inputTargetNode != null)
      "D2B_AUDIO_INPUT_TARGET_NODE=${cfg.site.audio.inputTargetNode}";
  };

  vsockRelayRunner = name: manifest: {
    binaryPath = "${cfg.observability.transport.relayPackage}/bin/socat";
    argv = [
      "d2b-otel-relay@${name}"
      "-d"
      "-d"
      "UNIX-LISTEN:${vsockSocketForPort manifest.observability.vsockHostSocket obsOtlpPort},fork,max-children=16,reuseaddr,mode=0660"
      "EXEC:${chVsockConnect}/bin/d2b-ch-vsock-connect ${cfg.store.stateDir}/${cfg.observability.vmName}/vsock.sock ${toString (obsSourcePort name)}"
    ];
  };

  otelHostBridgeRunner = manifest: {
    binaryPath = "${cfg.observability.transport.relayPackage}/bin/socat";
    argv = [
      "d2b-otel-host-bridge"
      "-d"
      "-d"
      "UNIX-LISTEN:/run/d2b/otel/host-egress.sock,fork,reuseaddr,mode=0660"
      ''EXEC:"${chVsockConnect}/bin/d2b-ch-vsock-connect ${manifest.observability.vsockHostSocket} ${toString obsOtlpPort}"''
    ];
  };

  usbipBackendRunner = envName: {
    binaryPath = "${pkgs.linuxPackages.usbip}/bin/usbipd";
    argv = [
      "d2b-sys-${envName}-usbipd-backend"
      "-4"
      "--tcp-port"
      (toString (backendPort envName))
    ];
  };

  usbipProxyRunner = envName: m: {
    binaryPath = "${pkgs.socat}/bin/socat";
    # Generic per-env L4 forwarder. It does not inspect USBIP frames or busids,
    # so single-busid revocation must not bounce this sidecar while other
    # same-env streams may be active; use host unbind plus targeted
    # conntrack/socket cleanup, or fail closed if the stream cannot be isolated.
    argv = [
      "d2b-sys-${envName}-usbipd-proxy"
      "TCP-LISTEN:3240,bind=${m.hostUplinkIp},fork,max-children=4,reuseaddr"
      "TCP:127.0.0.1:${toString (backendPort envName)}"
    ];
  };

  mkDiskInitPlanOp = { targetPath, sizeBytes, mode, ownerProfile, ifAbsent ? true }: {
    kind = "diskInit";
    inherit targetPath sizeBytes mode ifAbsent;
    ownerUid = ownerProfile.uid;
    ownerGid = ownerProfile.gid;
  };

  mkProcessNode = name: { id, role, readiness, unit ? null, binaryPath ? null, argv ? [ ], env ? [ ], planOps ? [ ] }:
    let
      # `vm.supervisor` was removed per ADR 0015; every
      # enabled VM is daemon-supervised. `emitUnit` is permanently
      # false so processes.json never reports a systemd unit
      # reference for a daemon-owned VM (preserves the single-
      # writer invariant). The retained `_` binding for the local
      # `vm` keeps the rest of the closure shape intact for
      # diff hygiene.
      _vm = cfg.vms.${name};
      emitUnit = false;
      emitRunner = binaryPath != null;
    in
    assert (binaryPath == null) == (argv == [ ]);
    {
      inherit id role readiness;
      profile = profileFor name id;
    }
    // lib.optionalAttrs emitUnit { inherit unit; }
    // lib.optionalAttrs emitRunner {
      inherit binaryPath argv;
    }
    // lib.optionalAttrs (env != [ ]) {
      inherit env;
    }
    // lib.optionalAttrs (planOps != [ ]) {
      inherit planOps;
    };

  mkRunnerNode = name: args: runner:
    mkProcessNode name (args // runner);

  runnerNode = name: { id, role, readiness, runner, unit ? null, planOps ? [ ] }:
    mkRunnerNode name {
      inherit id role readiness unit planOps;
    } runner;

  node = mkProcessNode;

  hypervisorRunnerNode = name: service: args:
    runnerNode name ({
      id = service.nodeId;
      role = service.runnerRole;
    } // args);

  mkEdge = from: to: reason: { inherit from to reason; };
  edge = mkEdge;
  edgesFromNodes = fromNodes: to: reason:
    builtins.map (from: edge from to reason) fromNodes;
  edgesToNodes = from: toNodes: reason:
    builtins.map (to: edge from to reason) toNodes;

  vmDag = name: vm:
    let
      manifest = cfg.manifest.${name};
      microvm = d2bLib.vmRunner config name;
      hypervisorService = d2bLib.runtimeHypervisorService "nixos";
      # The guest-control authenticated Health probe is the framework
      # readiness gate on guest-control-capable VMs. Per-VM sshd/host-keys are
      # retained for the SSH-compat window but are no longer the framework
      # readiness signal, so a TCP-22 readiness node is no longer emitted.
      guestControlEnabled = vm.guest.control.enable;
      virtiofsShares = lib.filter
        (share: (share.proto or "virtiofs") == "virtiofs")
        microvm.shares;
      shareNodes = lib.forEach virtiofsShares (share:
        mkRunnerNode name {
          id = shareNodeId share;
          role = "virtiofsd";
          readiness = [ (unixSocketExists (shareSocketPath name share)) ];
        } (virtiofsdRunner name share));
      shareNodeIds = builtins.map shareNodeId virtiofsShares;
      postStoreNodeIds = if shareNodeIds != [ ] then shareNodeIds else [ "store-virtiofs-preflight" ];
      preOptionalNodeIds = if vm.tpm.enable then [ "swtpm" ] else postStoreNodeIds;
      optionalSidecarBaseNodeIds = if vm.observability.enable then [ "vsock-relay" ] else preOptionalNodeIds;
      graphicsReadiness = [
        (unixSocketExists (d2bLib.vmRunner config name).graphics.socket)
      ] ++ lib.optional vm.graphics.virglVideo
        (componentReady "graphics.virglVideo=true");
      # Whether the host-jailed Wayland proxy is emitted for this VM.
      # Requires graphics.enable, crossDomainTrusted, and waylandProxy.enable.
      emitWaylandProxy = vm.graphics.enable && vm.graphics.crossDomainTrusted && vm.graphics.waylandProxy.enable;
      # Resolved GPU node id: gpu-render-node when renderNodeOnly is true.
      graphicsNodeId = if vm.graphics.renderNodeOnly then "gpu-render-node" else "gpu";
      preVmmNodeIds =
        lib.unique (
          (lib.optionals (vm.graphics.enable && vm.graphics.videoSidecar) [ "video" ])
          # When the wayland-proxy filter is active, GPU depends on
          # wayland-proxy (not directly on the gpu/render-node node — gpu
          # is behind wayland-proxy in the readiness chain). Cloud Hypervisor
          # waits on gpu/video directly, so preVmmNodeIds still names the
          # gpu/video node; the wayland-proxy edge is declared separately below.
          ++ (lib.optionals (vm.graphics.enable && !vm.graphics.videoSidecar) [ graphicsNodeId ])
          ++ (lib.optionals vm.audio.enable [ "audio" ])
          ++ lib.optionals (!vm.graphics.enable && !vm.audio.enable) optionalSidecarBaseNodeIds
        );
    in {
      vm = name;
      nodes = [
        (node name {
          id = "host-reconcile";
          role = "host-reconcile";
          readiness = [ (componentReady "host state, runtime directories, and bridges are reconciled") ];
        })
        (node name {
          id = "store-virtiofs-preflight";
          role = "store-virtiofs-preflight";
          unit = "d2b-${name}-store-sync.service";
          readiness = [
            (commandReady [ "test" "-e" "${toString cfg.store.stateDir}/${name}/store-view/live/.d2b-marker-${name}" ])
          ];
        })
      ]
      ++ lib.optional vm.tpm.enable (mkRunnerNode name {
        id = "swtpm-flush";
        role = "swtpm-pre-start-flush";
        readiness = [ ];
      } (swtpmFlushRunner name))
      ++ lib.optional vm.tpm.enable (mkRunnerNode name {
        id = "swtpm";
        role = "swtpm";
        readiness = [ (unixSocketListening manifest.tpmSocket) ];
      } (swtpmRunner name))
      ++ shareNodes
      ++ lib.optional (vm.graphics.enable && !vm.graphics.renderNodeOnly) (mkRunnerNode name {
        id = "gpu";
        role = "gpu";
      # (Option B): readiness uses microvm.graphics.socket
      # (the same path the argv tells crosvm to create), not the
      # stale /var/lib/d2b/vms/<vm>/<vm>-gpu.sock from manifest.
      # The two paths diverged when the v1.1.1 substrate moved the
      # gpu sidecar from /var/lib/d2b to /run/d2b without
      # updating the readiness predicate. Without this fix the DAG
      # times out waiting on a socket that crosvm never creates.
      readiness = graphicsReadiness;
      } (gpuRunner name vm))
      # (ADR 0021) render-node-only broker-pre-NS GPU sidecar.
      # Emitted when graphics.renderNodeOnly = true. Uses gpuRenderNodeRunner
      # (argv carries --gpu-device-node /proc/self/fd/10) and the
      # gpu-render-node minijail profile (userNamespace, empty deviceBinds).
      ++ lib.optional (vm.graphics.enable && vm.graphics.renderNodeOnly) (mkRunnerNode name {
        id = "gpu-render-node";
        role = "gpu-render-node";
        readiness = graphicsReadiness;
      } (gpuRenderNodeRunner name vm))
      ++ lib.optional (vm.graphics.enable && vm.graphics.videoSidecar) (mkRunnerNode name {
        id = "video";
        role = "video";
        readiness = [ (unixSocketListening "/run/d2b-video/${name}/video.sock") ];
      } (videoRunner name))
      ++ lib.optional emitWaylandProxy (mkRunnerNode name {
        id = "wayland-proxy";
        role = "wayland-proxy";
        readiness = [
          (unixSocketListening "/run/d2b-wlproxy/${name}/wayland-0")
        ];
      } (waylandProxyRunner name vm))
      ++ lib.optional vm.audio.enable (mkRunnerNode name {
        id = "audio";
        role = "audio";
        readiness = [ (unixSocketExists "/run/d2b/vms/${name}/snd.sock") ];
      } (audioRunner name))
      ++ [
        (hypervisorRunnerNode name hypervisorService {
          runner = {
            binaryPath = cloudHypervisorBinaryPath microvm;
            argv = cloudHypervisorArgv name vm manifest;
            # the cloud-hypervisor binary is a bash
            # wrapper that calls `dirname` to compute paths; under
            # the broker spawn (empty PATH) it exits 127 on the
            # very first line. Provide PATH with coreutils so the
            # wrapper can find dirname + sed.
            env = [
              "PATH=${pkgs.coreutils}/bin:${pkgs.gnused}/bin"
            ];
          };
          readiness = [ (apiSocketInfo manifest.apiSocket) ];
          # emit DiskInit plan-ops before SpawnRunner.
          # D2b-owned relative raw/ext4 microvm.volumes are declared by
          # the consumer and mounted inside the guest by vm-guest-base.nix,
          # so missing images must be created and mkfs'd before CH starts.
          # Existing images are validated non-destructively (`ifAbsent = true`):
          # the broker skips ext4 images, safely repairs stale declared
          # owner/mode posture after fd-bound identity checks, safely formats a
          # proven-empty image, and fails closed for ambiguous data.
          #
          # mode 0o660 = 432 decimal for regular VM volumes (CH runner
          # opens them via kvm group); store-overlay keeps 0o600.
          planOps = (builtins.map (volume: mkDiskInitPlanOp {
            targetPath = volumeHostPath name volume;
            sizeBytes = d2bLib.volumeSizeBytes volume;
            mode = 432;
            ownerProfile = profileFor name "cloud-hypervisor";
          }) (builtins.filter d2bLib.volumeDiskInitEligible microvm.volumes))
          ++ lib.optionals (microvm.writableStoreOverlay != null) [
            (mkDiskInitPlanOp {
              targetPath = "${toString cfg.store.stateDir}/${name}/store-overlay.img";
              sizeBytes = vm.writableStoreOverlaySize;
              mode = 384;
              ownerProfile = profileFor name "cloud-hypervisor";
            })
          ];
        })
      ]
      ++ lib.optional vm.observability.enable (runnerNode name {
        id = "vsock-relay";
        role = "vsock-relay";
        unit = "d2b-otel-relay@${name}.service";
        readiness = [ (unixSocketExists (vsockSocketForPort manifest.observability.vsockHostSocket obsOtlpPort)) ];
        runner = vsockRelayRunner name manifest;
      })
      ++ lib.optional (cfg.observability.enable && name == cfg.observability.vmName) (runnerNode name {
        id = "otel-host-bridge";
        role = "otel-host-bridge";
        unit = "d2b-otel-host-bridge.service";
        readiness = [ (unixSocketExists "/run/d2b/otel/host-egress.sock") ];
        runner = otelHostBridgeRunner manifest;
      })
      ++ lib.optional guestControlEnabled (node name {
        id = "guest-control-health";
        role = "guest-control-health";
        readiness = [ (guestControlHealthReady name) ];
      });
      edges = [
        (edge "host-reconcile" "store-virtiofs-preflight" "Host reconciliation must complete before store and virtiofs preflight runs.")
      ]
      ++ edgesToNodes "store-virtiofs-preflight" shareNodeIds "Each virtiofs share depends on the per-VM store view and marker preflight."
      ++ lib.optionals vm.tpm.enable (
        (edgesFromNodes postStoreNodeIds "swtpm-flush" "The swtpm pre-start flush runs only after every virtiofs share is ready.")
        ++ [ (edge "swtpm-flush" "swtpm" "The long-lived swtpm sidecar starts only after the one-shot flush completes.") ]
      )
      ++ lib.optionals vm.observability.enable (
        edgesFromNodes preOptionalNodeIds "vsock-relay" "The vsock relay starts only after runtime/state dirs, taps, cgroup setup, and earlier sidecars are ready."
      )
      ++ lib.optionals (cfg.observability.enable && name == cfg.observability.vmName) (
        [
          (edge "cloud-hypervisor" "otel-host-bridge" "The host OTel bridge starts only after the obs VM Cloud Hypervisor runner creates the base vsock socket.")
        ]
      )
      ++ lib.optionals vm.graphics.enable (
        (edgesFromNodes optionalSidecarBaseNodeIds graphicsNodeId "The GPU sidecar starts only after every prerequisite sidecar is ready.")
        ++ lib.optional emitWaylandProxy
          (edge "host-reconcile" "wayland-proxy" "The Wayland proxy starts only after host reconciliation prepares runtime directories and socket ACLs.")
        ++ lib.optional vm.graphics.videoSidecar
          (edge graphicsNodeId "video" "The optional video decoder sidecar depends on the GPU sidecar.")
        # GPU connects to the proxy socket, so wayland-proxy must be
        # listening before the GPU starts. Emit only when the proxy is
        # present.
        ++ lib.optional emitWaylandProxy
          (edge "wayland-proxy" graphicsNodeId "The GPU sidecar starts only after the Wayland proxy is listening on its socket.")
      )
      ++ lib.optionals vm.audio.enable (
        edgesFromNodes optionalSidecarBaseNodeIds "audio" "The audio sidecar starts only after every prerequisite sidecar is ready."
      )
      ++ edgesFromNodes preVmmNodeIds "cloud-hypervisor" "Cloud Hypervisor starts only after every prerequisite sidecar is ready."
      ++ lib.optional guestControlEnabled
        (edge "cloud-hypervisor" "guest-control-health" "Authenticated guest-control Health readiness is probed only after Cloud Hypervisor is running.");
      invariants = {
        perVmAuditPipeline = true;
        swtpmPreStartFlush = true;
        tpmOwnershipMigrationWithoutRunningVmMutation = true;
        usbipGating = true;
      };
    };

  qemuMediaDag = name: vm:
    let
      emitWaylandProxy = cfg.site.waylandUser != null;
      hypervisorService = d2bLib.runtimeHypervisorService "qemu-media";
    in
    {
      vm = name;
      nodes = [
        (node name {
          id = "host-reconcile";
          role = "host-reconcile";
          readiness = [ (componentReady "host state, runtime directories, and bridges are reconciled") ];
        })
      ] ++ lib.optional emitWaylandProxy (node name ({
        id = "wayland-proxy";
        role = "wayland-proxy";
        readiness = [
          (unixSocketListening "/run/d2b-wlproxy/${name}/wayland-0")
        ];
      } // waylandProxyRunner name vm))
      ++ [
        (hypervisorRunnerNode name hypervisorService {
          runner = {
            binaryPath = qemuMediaBinaryPath;
            argv = qemuMediaArgv name;
            env = qemuMediaEnv name;
          };
          readiness = [ (unixSocketListening (qemuMediaQmpSocket name)) ];
        })
      ];
      edges =
        if emitWaylandProxy then [
          (edge "host-reconcile" "wayland-proxy" "The Wayland proxy starts only after host reconciliation prepares runtime directories and socket ACLs.")
          (edge "wayland-proxy" "qemu-media" "QEMU media connects to the per-VM Wayland proxy instead of the host compositor socket.")
        ] else [
          (edge "host-reconcile" "qemu-media" "QEMU media starts only after host reconciliation prepares runtime directories and network state.")
        ];
      invariants = {
        perVmAuditPipeline = true;
        swtpmPreStartFlush = true;
        tpmOwnershipMigrationWithoutRunningVmMutation = true;
        usbipGating = true;
      };
    };

  usbipdDag = envName: m:
    let
      vmId = "sys-${envName}-usbipd";
    in {
      vm = vmId;
      nodes = [
        (runnerNode vmId {
          id = "backend";
          role = "usbip";
          readiness = [ (tcpPort "127.0.0.1" (backendPort envName)) ];
          runner = usbipBackendRunner envName;
        })
        (runnerNode vmId {
          id = "proxy";
          role = "usbip";
          readiness = [ (tcpPort m.hostUplinkIp 3240) ];
          runner = usbipProxyRunner envName m;
        })
      ];
      edges = [
        (edge "backend" "proxy" "The per-env USBIP proxy starts only after the backend usbipd listener is ready.")
      ];
      invariants = {
        perVmAuditPipeline = true;
        swtpmPreStartFlush = true;
        tpmOwnershipMigrationWithoutRunningVmMutation = true;
        usbipGating = true;
      };
    };

  data = {
    schemaVersion = "v2";
    vms =
      (lib.mapAttrsToList vmDag normalNixosVms)
      ++ (lib.mapAttrsToList qemuMediaDag qemuMediaVms)
      ++ (lib.mapAttrsToList usbipdDag cfg._index.usbip.envMeta);
  };

  jsonText = builtins.toJSON data;
  jsonFile = pkgs.writeText "d2b-processes.json" jsonText;
  videoAssertions = lib.flatten (lib.mapAttrsToList (name: vm:
    let
      manifest = cfg.manifest.${name};
      microvm = d2bLib.vmRunner config name;
      expectedMediaArg = "socket=/run/d2b-video/${name}/video.sock";
      values = mediaArgValues name vm manifest;
      flags = mediaFlagTokens name vm manifest;
    in
    lib.optionals (vm.enable && vm.graphics.videoSidecar) [
      {
        assertion = toString microvm.cloud-hypervisor.package == toString spectrumCH;
        message = ''
          d2b.vms.${name}.graphics.videoSidecar requires the vendored patched
          Cloud Hypervisor package from pkgs/spectrum-ch. Remove the
          microvm.cloud-hypervisor.package override or disable graphics.videoSidecar.
        '';
      }
      {
        assertion = flags == [ "--vhost-user-media" ] && values == [ expectedMediaArg ];
        message = ''
          d2b.vms.${name}.graphics.videoSidecar requires exactly one
          --vhost-user-media argument equal to ${expectedMediaArg}. Do not add
          or override media endpoints via microvm.cloud-hypervisor.extraArgs.
        '';
      }
    ]) normalNixosVms);
in
{
  config = {
    assertions = videoAssertions;
    d2b._bundle.processesJson = {
      inherit data jsonText;
      path = "${jsonFile}";
      installFileName = "processes.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };
}
