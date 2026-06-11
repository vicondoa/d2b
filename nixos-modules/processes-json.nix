{ config, lib, pkgs, ... }:

let
  clean = builtins.unsafeDiscardStringContext;

  cfg = config.nixling;
  # nixling-owned access helpers (see lib.nix).
  nl = import ./lib.nix { inherit lib pkgs; };
  enabledVms = lib.filterAttrs (_: vm: vm.enable) cfg.vms;
  usbipEnvNames = lib.sort lib.lessThan (lib.unique (lib.concatMap
    (vm: lib.optional (cfg.site.yubikey.enable && vm.enable && vm.usbip.yubikey && vm.env != null) vm.env)
    (lib.attrValues cfg.vms)));
  usbipMeta = lib.filterAttrs (envName: _: lib.elem envName usbipEnvNames) cfg._envMeta;
  obsOtlpPort = 14317;
  waylandUid =
    if cfg.site.waylandUser != null
    then toString (config.users.users.${cfg.site.waylandUser}.uid or 0)
    else "0";
  waylandDisplay = cfg.site.waylandDisplay;
  # Real host compositor socket. Used only by the wayland-proxy role;
  # GPU runners no longer reference this path directly.
  waylandHostSock = "/run/user/${waylandUid}/${waylandDisplay}";
  chVsockConnect = import ./nixling-ch-vsock-connect.nix { inherit pkgs; };
  vhostDeviceSound = import ../pkgs/vhost-device-sound { inherit pkgs; };
  spectrumCH = import ../pkgs/spectrum-ch { inherit pkgs; };

  # nixling-wayland-filter: host-side Wayland filter proxy.
  # Built from the workspace so the binary path is available for the
  # wayland-proxy DAG node's binaryPath field.
  packagesSrc = lib.cleanSourceWith {
    src = ../packages;
    filter = path: type:
      let rel = lib.removePrefix (toString ../packages + "/") (toString path);
      in !(lib.hasInfix "target" rel || lib.hasInfix ".cargo/registry" rel);
  };
  nixlingWaylandFilterPackage = pkgs.rustPlatform.buildRustPackage {
    pname = "nixling-wayland-filter";
    version = "0.0.0";
    src = packagesSrc;
    cargoLock = {
      lockFile = ../packages/Cargo.lock;
      outputHashes."wl-proxy-0.1.2" = "sha256-ZKXnOZwjRkt1lbQBpAQYrYKzn6rS4gje8YWE5ek4W/E=";
    };
    cargoBuildFlags = [ "--package" "nixling-wayland-filter" ];
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
      install -Dm755 target/x86_64-unknown-linux-gnu/release/nixling-wayland-filter $out/bin/nixling-wayland-filter 2>/dev/null \
        || install -Dm755 target/release/nixling-wayland-filter $out/bin/nixling-wayland-filter
      runHook postInstall
    '';
  };
  nixlingWaylandFilterBinary = "${nixlingWaylandFilterPackage}/bin/nixling-wayland-filter";

  envPortMap = lib.listToAttrs (
    lib.imap0 (i: envName: {
      name = envName;
      value = 3241 + i;
    }) (lib.attrNames cfg.envs)
  );
  backendPort = envName: envPortMap.${envName};

  privateEtc = source: {
    inherit source;
    mode = "0640";
    user = "root";
    group = if cfg.daemonExperimental.enable then "nixlingd" else "root";
  };

  profileIdFor = name: nodeId: "vm-${name}-${nodeId}";
  profileFor = name: nodeId:
    if nodeId == "otel-host-bridge"
    then cfg._bundle.minijailProfiles."host-otel-host-bridge".roleProfile
    else cfg._bundle.minijailProfiles.${profileIdFor name nodeId}.roleProfile;
  vsockSocketForPort = socketPath: port: "${socketPath}_${toString port}";
  shareNodeId = share: "virtiofsd-${clean share.tag}";
  shareSocketPath = name: share: "/run/nixling/vms/${name}/${clean share.tag}.sock";
  volumeHostPath = name: volume: nl.volumeHostPath cfg.store.stateDir name volume;

  componentReady = value: { kind = "component-specific"; inherit value; };
  apiSocketInfo = value: { kind = "api-socket-info"; inherit value; };
  unixSocketExists = value: { kind = "unix-socket-exists"; inherit value; };
  unixSocketListening = value: { kind = "unix-socket-listening"; inherit value; };
  tcpPort = host: port: { kind = "tcp-port"; value = { inherit host port; }; };
  commandReady = value: { kind = "command"; inherit value; };

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
    pkgs.writeShellScript "nixling-${name}-swtpm-flush" ''
      set -eu
      state_dir=/var/lib/nixling/vms/${name}/swtpm
      permall_file="$state_dir/swtpm_perm.state"
      flush_sock=/run/nixling/vms/${name}/tpm-flush.sock

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

  cloudHypervisorArgv = name: vm: manifest:
    let
      microvm = nl.vmRunner config name;
      extraArgs = microvm.cloud-hypervisor.extraArgs;
      processedExtraArgs = builtins.foldl'
        (args: opt: (extractOptValues opt args).args)
        extraArgs
        [ "--vsock" "--platform" ];
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
      userVSockOpts = (extractOptValues "--vsock" extraArgs).values;
      userVSockStr = if userVSockOpts == [ ] then null else builtins.head userVSockOpts;
      userVSockPath = extractParamValue "socket" userVSockStr;
      userVSockCID = extractParamValue "cid" userVSockStr;
      vsockCID =
        if microvm.vsock.cid != null && userVSockCID != null then
          throw "Cannot set microvm.vsock.cid and --vsock cid= ... via microvm.cloud-hypervisor.extraArgs at the same time"
        else if microvm.vsock.cid != null then
          microvm.vsock.cid
        else
          userVSockCID;
      supportsNotifySocket = vsockCID != null;
      vsockPath = if userVSockPath != null then userVSockPath else "/var/lib/nixling/vms/${name}/notify.vsock";
      vsockOpts =
        if vsockCID == null then
          null
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
            # omit. The nixling-owned vm-options.nix
            # types `microvm.volumes` as `listOf attrs` (untyped)
            # for forward compat; processes-json.nix supplies the
            # CH defaults when the consumer doesn't.
            direct = if (volume.direct or false) then "on" else "off";
            readonly = if (volume.readOnly or false) then "on" else "off";
            image_type = toString (volume.imageType or "raw");
            serial = nl.volumeSerial volume;
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
        "socket=/run/nixling/vms/${name}/snd.sock,virtio_id=25,queue_sizes=[64,64,64,64]"
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
    ++ lib.optionals (vsockOpts != null) [ "--vsock" vsockOpts ]
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
      microvm = nl.vmRunner config name;
      # (ADR 0021): under broker-pre-NS, virtiofsd is
      # fake-root inside its own user namespace. Use --sandbox=chroot
      # (now works because we have CAP_SYS_ADMIN inside the NS) and
      # disable file handles (we don't need open_by_handle_at(2)
      # for read-only or per-VM share serving). --posix-acl + --xattr
      # are dropped: /nix/store has no ACLs, and the per-VM shares
      # are nixling-managed (no foreign xattrs to preserve).
      isRoStore = share.source == "/nix/store";
      # SECURITY (per-VM store isolation): the ro-store share's guest
      # `/nix/store` must expose ONLY this VM's closure, never the host's
      # full `/nix/store`. `share.source` stays `/nix/store` as the
      # eval-time sentinel that the guest-mount + overlay + readiness
      # logic keys off, but virtiofsd is pointed at the per-VM hardlink
      # farm `<stateDir>/<vm>/store` — the canonical closure-only per-VM
      # store (see AGENTS.md "Per-VM /nix/store hardlink farm" +
      # nixos-modules/store.nix). virtiofsd still execs from the real host
      # `/nix/store` (kept mounted in its runner namespace) and only
      # *serves* the farm, so the guest sees a closure-only store. This
      # replaces the previous `--shared-dir=/nix/store`, which leaked the
      # host's entire store into every guest. Mirrors the legacy
      # `BindReadOnlyPaths /nix/store -> per-VM farm` behaviour.
      roStoreSharedDir = "${toString cfg.store.stateDir}/${name}/store";
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
      ++ lib.optionals (isRoStore || (share.readOnly or false)) [ "--readonly" ]
      ++ microvm.virtiofsd.extraArgs;
    };

  swtpmFlushRunner = name:
    let
      script = swtpmFlushScript name;
    in {
      binaryPath = "${script}";
      argv = [ "nixling-swtpm-flush@${name}" ];
    };

  swtpmRunner = name: {
    binaryPath = "${pkgs.swtpm}/bin/swtpm";
    argv = [
      "microvm-swtpm@${name}"
      "socket"
      "--tpmstate"
      "dir=/var/lib/nixling/vms/${name}/swtpm"
      "--ctrl"
      # mode=0660 (was 0600) so that combined with
      # umask 0o007 from the swtpm role profile and the per-VM
      # /run/nixling/vms/<vm>/ default ACL granting CH's UID rw,
      # cloud-hypervisor can connect to the TPM control socket
      # without operator setfacl intervention.
      "type=unixio,path=/run/nixling/vms/${name}/tpm.sock,mode=0660"
      "--tpm2"
      "--flags"
      "startup-clear"
    ];
  };

  gpuRunner = name: vm:
    let
      microvm = nl.vmRunner config name;
      gpuParams = "{\"context-types\":\"virgl:virgl2:cross-domain\",\"displays\":[{\"hidden\":true}],\"egl\":true,\"vulkan\":true}";
      filterSock = "/run/nixling-wlproxy/${name}/wayland-0";
      emitWaylandProxy = vm.graphics.enable && vm.graphics.crossDomainTrusted && vm.graphics.waylandFilter.enable;
      # When the filter proxy is emitted, crosvm connects to the filter
      # socket. Otherwise preserve the legacy display backend by connecting
      # directly to the real host compositor socket.
      waylandSock = if emitWaylandProxy then filterSock else waylandHostSock;
    in {
      binaryPath = "${microvm.graphics.crosvmPackage}/bin/crosvm";
      argv = [
        "nixling-${name}-gpu"
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
      microvm = nl.vmRunner config name;
      gpuParams = "{\"context-types\":\"virgl:virgl2:cross-domain\",\"displays\":[{\"hidden\":true}],\"egl\":true,\"vulkan\":true}";
      filterSock = "/run/nixling-wlproxy/${name}/wayland-0";
      emitWaylandProxy = vm.graphics.enable && vm.graphics.crossDomainTrusted && vm.graphics.waylandFilter.enable;
      waylandSock = if emitWaylandProxy then filterSock else waylandHostSock;
    in {
      binaryPath = "${microvm.graphics.crosvmPackage}/bin/crosvm";
      argv = [
        "nixling-${name}-gpu-render-node"
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

  # wayland-proxy runner: nixling-wayland-filter host-side filter proxy.
  # Runs as nixling-<vm>-wlproxy, listens on the per-VM filter socket,
  # and connects upstream to the real host compositor socket. The broker
  # grants the wlproxy principal an ACL on exactly that socket.
  waylandProxyRunner = name: vm:
    let
      vmName = name;
      filterSock = "/run/nixling-wlproxy/${vmName}/wayland-0";
      upstreamSock = waylandHostSock;
      appIdPrefix = "nixling.${vmName}.";
      titlePrefix = "[${vmName}] ";
      denyArgs = lib.concatMap (g: [ "--deny-global" g ]) vm.graphics.waylandFilter.denyGlobals;
      allowArgs = lib.concatMap (g: [ "--allow-global" g ]) vm.graphics.waylandFilter.allowGlobals;
      maxVersionArgs = lib.concatMap
        (nameVersion:
          let parts = lib.splitString "=" nameVersion;
          in [ "--max-version" nameVersion ])
        (lib.mapAttrsToList (iface: ver: "${iface}=${toString ver}") vm.graphics.waylandFilter.maxVersions);
    in {
      binaryPath = nixlingWaylandFilterBinary;
      env = lib.optionals vm.graphics.waylandFilter.debugLogging [
        "WL_PROXY_DEBUG=1"
        "WL_PROXY_PREFIX=nixling-${vmName}-wlproxy"
      ];
      argv = [
        "nixling-${vmName}-wlproxy"
        "--listen" filterSock
        "--connect" upstreamSock
        "--vm-name" vmName
        "--app-id-prefix" appIdPrefix
        "--title-prefix" titlePrefix
      ] ++ denyArgs ++ allowArgs ++ maxVersionArgs;
    };

  videoBinaryPath = _name:
    # the per-VM
    # `nixling-${name}-video.service` was deleted. The video
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
      "nixling-${name}-video"
      "device"
      "video-decoder"
      "--socket-path"
      "/run/nixling-video/${name}/video.sock"
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
      "nixling-${name}-snd"
      "--socket"
      "/run/nixling/vms/${name}/snd.sock"
      "--backend"
      "pipewire"
    ];
    # (Option B): point libpipewire at the Wayland
    # user's PipeWire socket. Without these env vars,
    # vhost-device-sound (running as the ephemeral role UID)
    # looks at /run/user/$EUID/pipewire-0 which doesn't exist
    # for the ephemeral UID. The PipeWire socket itself is
    # grant'd to the ephemeral UID by host-activation.nix's
    # nixlingRoleUidAcls script.
    env = [
      "PIPEWIRE_RUNTIME_DIR=/run/user/${waylandUid}"
      "XDG_RUNTIME_DIR=/run/user/${waylandUid}"
      ''PIPEWIRE_PROPS={ application.name = "nixling-${name}" node.name = "nixling-${name}" node.description = "nixling ${name}" nixling.vm = "${name}" }''
    ] ++ lib.optional (cfg.site.audio.inputTargetNode != null)
      "NIXLING_AUDIO_INPUT_TARGET_NODE=${cfg.site.audio.inputTargetNode}";
  };

  vsockRelayRunner = name: manifest: {
    binaryPath = "${cfg.observability.transport.relayPackage}/bin/socat";
    argv = [
      "nixling-otel-relay@${name}"
      "-d"
      "-d"
      "UNIX-LISTEN:${vsockSocketForPort manifest.observability.vsockHostSocket obsOtlpPort},fork,max-children=16,reuseaddr,mode=0660"
      "EXEC:${chVsockConnect}/bin/nixling-ch-vsock-connect ${cfg.store.stateDir}/${cfg.observability.vmName}/vsock.sock ${toString obsOtlpPort}"
    ];
  };

  otelHostBridgeRunner = manifest: {
    binaryPath = "${cfg.observability.transport.relayPackage}/bin/socat";
    argv = [
      "nixling-otel-host-bridge"
      "-d"
      "-d"
      "UNIX-LISTEN:/run/nixling/otel/host-egress.sock,fork,reuseaddr,mode=0660"
      ''EXEC:"${chVsockConnect}/bin/nixling-ch-vsock-connect ${manifest.observability.vsockHostSocket} ${toString obsOtlpPort}"''
    ];
  };

  usbipBackendRunner = envName: {
    binaryPath = "${pkgs.linuxPackages.usbip}/bin/usbipd";
    argv = [
      "nixling-sys-${envName}-usbipd-backend"
      "-4"
      "--tcp-port"
      (toString (backendPort envName))
    ];
  };

  usbipProxyRunner = envName: m: {
    binaryPath = "${pkgs.socat}/bin/socat";
    argv = [
      "nixling-sys-${envName}-usbipd-proxy"
      "TCP-LISTEN:3240,bind=${m.hostUplinkIp},fork,max-children=4,reuseaddr"
      "TCP:127.0.0.1:${toString (backendPort envName)}"
    ];
  };

  node = name: { id, role, readiness, unit ? null, binaryPath ? null, argv ? [ ], env ? [ ], planOps ? [ ] }:
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

  edge = from: to: reason: { inherit from to reason; };
  edgesFromNodes = fromNodes: to: reason:
    builtins.map (from: edge from to reason) fromNodes;
  edgesToNodes = from: toNodes: reason:
    builtins.map (to: edge from to reason) toNodes;

  vmDag = name: vm:
    let
      manifest = cfg.manifest.${name};
      microvm = nl.vmRunner config name;
      guestSshEnabled = manifest.sshUser != null && manifest.staticIp != null;
      virtiofsShares = lib.filter
        (share: (share.proto or "virtiofs") == "virtiofs")
        microvm.shares;
      shareNodes = lib.forEach virtiofsShares (share:
        node name ({
          id = shareNodeId share;
          role = "virtiofsd";
          unit = "microvm-virtiofsd@${name}.service";
          readiness = [ (unixSocketExists (shareSocketPath name share)) ];
        } // virtiofsdRunner name share));
      shareNodeIds = builtins.map shareNodeId virtiofsShares;
      postStoreNodeIds = if shareNodeIds != [ ] then shareNodeIds else [ "store-virtiofs-preflight" ];
      preOptionalNodeIds = if vm.tpm.enable then [ "swtpm" ] else postStoreNodeIds;
      optionalSidecarBaseNodeIds = if vm.observability.enable then [ "vsock-relay" ] else preOptionalNodeIds;
      graphicsReadiness = [
        (unixSocketExists (nl.vmRunner config name).graphics.socket)
      ] ++ lib.optional vm.graphics.virglVideo
        (componentReady "graphics.virglVideo=true");
      # Whether the host-jailed Wayland filter proxy is emitted for this VM.
      # Requires graphics.enable, crossDomainTrusted, and waylandFilter.enable.
      emitWaylandProxy = vm.graphics.enable && vm.graphics.crossDomainTrusted && vm.graphics.waylandFilter.enable;
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
          unit = "nixling-${name}-store-sync.service";
          readiness = [
            (commandReady [ "test" "-e" "${toString cfg.store.stateDir}/${name}/store/.nixling-marker-${name}" ])
          ];
        })
      ]
      ++ lib.optional vm.tpm.enable (node name ({
        id = "swtpm-flush";
        role = "swtpm-pre-start-flush";
        unit = "nixling-${name}-swtpm.service";
        readiness = [ ];
      } // swtpmFlushRunner name))
      ++ lib.optional vm.tpm.enable (node name ({
        id = "swtpm";
        role = "swtpm";
        unit = "nixling-${name}-swtpm.service";
        readiness = [ (unixSocketExists manifest.tpmSocket) ];
      } // swtpmRunner name))
      ++ shareNodes
      ++ lib.optional (vm.graphics.enable && !vm.graphics.renderNodeOnly) (node name ({
        id = "gpu";
        role = "gpu";
        unit = "nixling-${name}-gpu.service";
      # (Option B): readiness uses microvm.graphics.socket
      # (the same path the argv tells crosvm to create), not the
      # stale /var/lib/nixling/vms/<vm>/<vm>-gpu.sock from manifest.
      # The two paths diverged when the v1.1.1 substrate moved the
      # gpu sidecar from /var/lib/nixling to /run/nixling without
      # updating the readiness predicate. Without this fix the DAG
      # times out waiting on a socket that crosvm never creates.
      readiness = graphicsReadiness;
      } // gpuRunner name vm))
      # (ADR 0021) render-node-only broker-pre-NS GPU sidecar.
      # Emitted when graphics.renderNodeOnly = true. Uses gpuRenderNodeRunner
      # (argv carries --gpu-device-node /proc/self/fd/10) and the
      # gpu-render-node minijail profile (userNamespace, empty deviceBinds).
      ++ lib.optional (vm.graphics.enable && vm.graphics.renderNodeOnly) (node name ({
        id = "gpu-render-node";
        role = "gpu-render-node";
        unit = "nixling-${name}-gpu.service";
        readiness = graphicsReadiness;
      } // gpuRenderNodeRunner name vm))
      ++ lib.optional (vm.graphics.enable && vm.graphics.videoSidecar) (node name ({
        id = "video";
        role = "video";
        readiness = [ (unixSocketListening "/run/nixling-video/${name}/video.sock") ];
      } // videoRunner name))
      ++ lib.optional emitWaylandProxy (node name ({
        id = "wayland-proxy";
        role = "wayland-proxy";
        readiness = [
          (unixSocketListening "/run/nixling-wlproxy/${name}/wayland-0")
        ];
      } // waylandProxyRunner name vm))
      ++ lib.optional vm.audio.enable (node name ({
        id = "audio";
        role = "audio";
        unit = "nixling-${name}-snd.service";
        readiness = [ (unixSocketExists "/run/nixling/vms/${name}/snd.sock") ];
      } // audioRunner name))
      ++ [
        (node name {
          id = "cloud-hypervisor";
          role = "cloud-hypervisor-runner";
          unit = if vm.graphics.enable then "nixling-${name}-gpu.service" else "microvm@${name}.service";
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
          readiness = [ (apiSocketInfo manifest.apiSocket) ];
          # emit DiskInit plan-ops before SpawnRunner.
          # Nixling-owned relative raw/ext4 microvm.volumes are declared by
          # the consumer and mounted inside the guest by vm-guest-base.nix,
          # so missing images must be created and mkfs'd before CH starts.
          # Existing images are skipped non-destructively (`ifAbsent = true`).
          #
          # mode 0o660 = 432 decimal for regular VM volumes (CH runner
          # opens them via kvm group); store-overlay keeps 0o600.
          planOps = (builtins.map (volume: {
            kind = "diskInit";
            targetPath = volumeHostPath name volume;
            sizeBytes = nl.volumeSizeBytes volume;
            mode = 432;
            ownerUid = (profileFor name "cloud-hypervisor").uid;
            ownerGid = (profileFor name "cloud-hypervisor").gid;
            ifAbsent = true;
          }) (builtins.filter nl.volumeDiskInitEligible microvm.volumes))
          ++ lib.optionals (microvm.writableStoreOverlay != null) [
            {
              kind = "diskInit";
              targetPath = "${toString cfg.store.stateDir}/${name}/store-overlay.img";
              sizeBytes = vm.writableStoreOverlaySize;
              mode = 384;
              ownerUid = (profileFor name "cloud-hypervisor").uid;
              ownerGid = (profileFor name "cloud-hypervisor").gid;
              ifAbsent = true;
            }
          ];
        })
      ]
      ++ lib.optional vm.observability.enable (node name ({
        id = "vsock-relay";
        role = "vsock-relay";
        unit = "nixling-otel-relay@${name}.service";
        readiness = [ (unixSocketExists (vsockSocketForPort manifest.observability.vsockHostSocket obsOtlpPort)) ];
      } // vsockRelayRunner name manifest))
      ++ lib.optional (cfg.observability.enable && name == cfg.observability.vmName) (node name ({
        id = "otel-host-bridge";
        role = "otel-host-bridge";
        unit = "nixling-otel-host-bridge.service";
        readiness = [ (unixSocketExists "/run/nixling/otel/host-egress.sock") ];
      } // otelHostBridgeRunner manifest))
      ++ lib.optional guestSshEnabled (node name {
        id = "guest-ssh-readiness";
        role = "guest-ssh-readiness";
        readiness = [ (tcpPort manifest.staticIp 22) ];
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
          (edge "host-reconcile" "wayland-proxy" "The Wayland filter proxy starts only after host reconciliation prepares runtime directories and socket ACLs.")
        ++ lib.optional vm.graphics.videoSidecar
          (edge graphicsNodeId "video" "The optional video decoder sidecar depends on the GPU sidecar.")
        # GPU connects to the filter socket, so wayland-proxy must be
        # listening before the GPU starts. Emit only when the proxy is
        # present.
        ++ lib.optional emitWaylandProxy
          (edge "wayland-proxy" graphicsNodeId "The GPU sidecar starts only after the Wayland filter proxy is listening on its socket.")
      )
      ++ lib.optionals vm.audio.enable (
        edgesFromNodes optionalSidecarBaseNodeIds "audio" "The audio sidecar starts only after every prerequisite sidecar is ready."
      )
      ++ edgesFromNodes preVmmNodeIds "cloud-hypervisor" "Cloud Hypervisor starts only after every prerequisite sidecar is ready."
      ++ lib.optional guestSshEnabled
        (edge "cloud-hypervisor" "guest-ssh-readiness" "SSH readiness is checked only after Cloud Hypervisor is running.");
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
        (node vmId ({
          id = "backend";
          role = "usbip";
          readiness = [ (tcpPort "127.0.0.1" (backendPort envName)) ];
        } // usbipBackendRunner envName))
        (node vmId ({
          id = "proxy";
          role = "usbip";
          readiness = [ (tcpPort m.hostUplinkIp 3240) ];
        } // usbipProxyRunner envName m))
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
    vms = (lib.mapAttrsToList vmDag enabledVms) ++ (lib.mapAttrsToList usbipdDag usbipMeta);
  };

  jsonText = builtins.toJSON data;
  jsonFile = pkgs.writeText "nixling-processes.json" jsonText;
  videoAssertions = lib.flatten (lib.mapAttrsToList (name: vm:
    let
      manifest = cfg.manifest.${name};
      microvm = nl.vmRunner config name;
      expectedMediaArg = "socket=/run/nixling-video/${name}/video.sock";
      values = mediaArgValues name vm manifest;
      flags = mediaFlagTokens name vm manifest;
    in
    lib.optionals (vm.enable && vm.graphics.videoSidecar) [
      {
        assertion = toString microvm.cloud-hypervisor.package == toString spectrumCH;
        message = ''
          nixling.vms.${name}.graphics.videoSidecar requires the vendored patched
          Cloud Hypervisor package from pkgs/spectrum-ch. Remove the
          microvm.cloud-hypervisor.package override or disable graphics.videoSidecar.
        '';
      }
      {
        assertion = flags == [ "--vhost-user-media" ] && values == [ expectedMediaArg ];
        message = ''
          nixling.vms.${name}.graphics.videoSidecar requires exactly one
          --vhost-user-media argument equal to ${expectedMediaArg}. Do not add
          or override media endpoints via microvm.cloud-hypervisor.extraArgs.
        '';
      }
    ]) enabledVms);
in
{
  options.nixling._bundle.processesJson = lib.mkOption {
    type = lib.types.unspecified;
    readOnly = true;
    internal = true;
    description = "Internal schema-v1 processes.json artifact metadata.";
  };

  config = {
    assertions = videoAssertions;
    nixling._bundle.processesJson = {
      inherit data jsonText;
      path = "${jsonFile}";
    };
    environment.etc."nixling/processes.json" = privateEtc jsonFile;
  };
}
