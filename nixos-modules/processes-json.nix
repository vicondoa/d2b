{ config, lib, pkgs, ... }:

let
  clean = builtins.unsafeDiscardStringContext;

  cfg = config.nixling;
  # v1.1-P8: nixling-owned access helpers (see lib.nix).
  nl = import ./lib.nix { inherit lib pkgs; };
  enabledVms = lib.filterAttrs (_: vm: vm.enable) cfg.vms;
  obsOtlpPort = 14317;
  waylandUid =
    if cfg.site.waylandUser != null
    then toString (config.users.users.${cfg.site.waylandUser}.uid or 0)
    else "0";
  chVsockConnect = import ./nixling-ch-vsock-connect.nix { inherit pkgs; };
  vhostDeviceSound = import ../pkgs/vhost-device-sound { inherit pkgs; };
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
  profileFor = name: nodeId: cfg._bundle.minijailProfiles.${profileIdFor name nodeId}.roleProfile;
  vsockSocketForPort = socketPath: port: "${socketPath}_${toString port}";
  shareNodeId = share: "virtiofsd-${clean share.tag}";
  shareSocketPath = name: share: "/run/nixling/vms/${name}/${clean share.tag}.sock";

  componentReady = value: { kind = "component-specific"; inherit value; };
  apiSocketInfo = value: { kind = "api-socket-info"; inherit value; };
  unixSocketExists = value: { kind = "unix-socket-exists"; inherit value; };
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

  # v1.1.2fu30: CH 52 changed `--fs`/`--net`/`--disk`/`--device` from
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
          throw "Cannot set microvm.vsock.cid and --vsock cid=... via microvm.cloud-hypervisor.extraArgs at the same time"
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
            # v1.1.1fu13g: prefix relative volume.image with the
            # per-VM state dir so CH (which has no cwd under broker
            # spawn) can find the disk. Absolute paths pass through
            # unchanged.
            path = let s = toString volume.image; in
              if lib.hasPrefix "/" s then s
              else "${toString cfg.store.stateDir}/${name}/${s}";
            # v1.1-final: defensive defaults for volume fields the
            # consumer may omit. The nixling-owned vm-options.nix
            # types `microvm.volumes` as `listOf attrs` (untyped)
            # for forward compat; processes-json.nix supplies the
            # CH defaults when the consumer doesn't.
            direct = if (volume.direct or false) then "on" else "off";
            readonly = if (volume.readOnly or false) then "on" else "off";
            image_type = toString (volume.imageType or "raw");
          }
          // lib.optionalAttrs ((volume.serial or null) != null) {
            serial = volume.serial;
          }
          // diskMqOps)
        ) microvm.volumes
        ++ lib.optionals (microvm.writableStoreOverlay != null) [
          (opsMapped {
            # v1.1.1fu13j: writableStoreOverlay is a guest-side
            # overlayfs upper layer. Under microvm.nix v1 it lived
            # on the host at /var/lib/microvms/<vm>/store-overlay.img
            # (a separate raw disk). The broker-spawn migration
            # doesn't create that backing file. Disable the disk
            # arg until a per-VM overlay-backing-image creation step
            # is in place (the guest still has var.img for writable
            # storage; only nix-env / ad-hoc store mutations need
            # the overlay).
            path = "${toString cfg.store.stateDir}/${name}/store-overlay.img";
            serial = "rootfs";
            readonly = "off";
          })
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
        # v1.1-final: defensive default for device.bus when the
        # consumer doesn't specify (defaults to "pci" for the
        # v1.0 contract preserved here).
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

  virtiofsdRunner = name: share:
    let
      microvm = nl.vmRunner config name;
      # v1.1.1fu14 (ADR 0021): under broker-pre-NS, virtiofsd is
      # fake-root inside its own user namespace. Use --sandbox=chroot
      # (now works because we have CAP_SYS_ADMIN inside the NS) and
      # disable file handles (we don't need open_by_handle_at(2)
      # for read-only or per-VM share serving). --posix-acl + --xattr
      # are dropped: /nix/store has no ACLs, and the per-VM shares
      # are nixling-managed (no foreign xattrs to preserve).
      isRoStore = share.source == "/nix/store";
    in {
      binaryPath = "${microvm.virtiofsd.package}/bin/virtiofsd";
      argv = [
        "microvm-virtiofsd@${name}-${clean share.tag}"
        "--socket-path=${shareSocketPath name share}"
      ]
      ++ lib.optionals (microvm.virtiofsd.group != null) [ "--socket-group=${microvm.virtiofsd.group}" ]
      ++ [
        "--shared-dir=${toString share.source}"
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
      # v1.1.2fu36: mode=0660 (was 0600) so that combined with
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

  gpuRunner = name:
    let
      microvm = nl.vmRunner config name;
      gpuParams = "{\"context-types\":\"virgl:virgl2:cross-domain\",\"displays\":[{\"hidden\":true}],\"egl\":true,\"vulkan\":true}";
    in {
      binaryPath = "${microvm.graphics.crosvmPackage}/bin/crosvm";
      argv = [
        "nixling-${name}-gpu"
        "device"
        "gpu"
        "--socket"
        microvm.graphics.socket
        "--wayland-sock"
        "/run/user/${waylandUid}/wayland-0"
        "--params"
        gpuParams
      ];
      # v1.1.1fu11 (Option B): Wayland + XDG runtime for crosvm
      # GPU sidecar. The --wayland-sock path is also in argv so
      # crosvm knows where to bind; XDG_RUNTIME_DIR is for
      # libwayland's auto-discovery + temporary state.
      env = [
        "XDG_RUNTIME_DIR=/run/user/${waylandUid}"
        "WAYLAND_DISPLAY=wayland-0"
      ];
    };

  videoBinaryPath = _name:
    # P6 ph6-remove-systemd-emission: the per-VM
    # `nixling-${name}-video.service` was deleted. The video
    # sidecar is now broker-spawned via SpawnRunner{role: Video},
    # and the broker takes the binary path from the bundle's
    # helperPaths map (populated by this file's emitter below).
    # We construct the crosvmVideo binary path inline using the
    # same overlay the deleted systemd template used.
    "${crosvmVideo}/bin/crosvm";

  # P6 ph6-remove-systemd-emission: crosvmVideo derivation
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
    cargoCheckFeatures = (old.cargoCheckFeatures or old.cargoBuildFeatures or old.buildFeatures or []) ++ [
      "video-decoder" "vaapi" "media"
    ];
    postPatch = (old.postPatch or "") + ''
      mkdir -p devices/src/virtio/vhost_user_backend/video/sys
      cp ${../pkgs/vhost-user-video/mod.rs} devices/src/virtio/vhost_user_backend/video/mod.rs
      cp ${../pkgs/vhost-user-video/sys_mod.rs} devices/src/virtio/vhost_user_backend/video/sys/mod.rs
      cp ${../pkgs/vhost-user-video/sys_linux.rs} devices/src/virtio/vhost_user_backend/video/sys/linux.rs
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
    # v1.1.1fu11 (Option B): video sidecar uses vaapi which
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
    # v1.1.1fu11 (Option B): point libpipewire at the Wayland
    # user's PipeWire socket. Without these env vars,
    # vhost-device-sound (running as the ephemeral role UID)
    # looks at /run/user/$EUID/pipewire-0 which doesn't exist
    # for the ephemeral UID. The PipeWire socket itself is
    # grant'd to the ephemeral UID by host-activation.nix's
    # nixlingRoleUidAcls script.
    env = [
      "PIPEWIRE_RUNTIME_DIR=/run/user/${waylandUid}"
      "XDG_RUNTIME_DIR=/run/user/${waylandUid}"
    ];
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

  usbipRunner = manifest: {
    binaryPath = "${pkgs.systemd}/lib/systemd/systemd-socket-proxyd";
    argv = [
      "nixling-sys-${manifest.env}-usbipd-proxy"
      "127.0.0.1:${toString (backendPort manifest.env)}"
    ];
  };

  node = name: { id, role, readiness, unit ? null, binaryPath ? null, argv ? [ ], env ? [ ] }:
    let
      # v1.1-P2: `vm.supervisor` was removed per ADR 0015; every
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
      usbipEnabled = vm.usbip.yubikey && guestSshEnabled && manifest.usbipdHostIp != null;
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
      preVmmNodeIds =
        lib.unique (
          (lib.optionals (vm.graphics.enable && vm.graphics.videoSidecar) [ "video" ])
          ++ (lib.optionals (vm.graphics.enable && !vm.graphics.videoSidecar) [ "gpu" ])
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
      ++ lib.optional vm.graphics.enable (node name ({
        id = "gpu";
        role = "gpu";
        unit = "nixling-${name}-gpu.service";
      # v1.1.1fu11 (Option B): readiness uses microvm.graphics.socket
      # (the same path the argv tells crosvm to create), not the
      # stale /var/lib/nixling/vms/<vm>/<vm>-gpu.sock from manifest.
      # The two paths diverged when the v1.1.1 substrate moved the
      # gpu sidecar from /var/lib/nixling to /run/nixling without
      # updating the readiness predicate. Without this fix the DAG
      # times out waiting on a socket that crosvm never creates.
      readiness = [ (unixSocketExists (nl.vmRunner config name).graphics.socket) ];
      } // gpuRunner name))
      ++ lib.optional (vm.graphics.enable && vm.graphics.videoSidecar) (node name ({
        id = "video";
        role = "video";
        unit = "nixling-${name}-video.service";
        readiness = [ (unixSocketExists "/run/nixling-video/${name}/video.sock") ];
      } // videoRunner name))
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
          # v1.1.1fu13d: the cloud-hypervisor binary is a bash
          # wrapper that calls `dirname` to compute paths; under
          # the broker spawn (empty PATH) it exits 127 on the
          # very first line. Provide PATH with coreutils so the
          # wrapper can find dirname + sed.
          env = [
            "PATH=${pkgs.coreutils}/bin:${pkgs.gnused}/bin"
          ];
          readiness = [ (apiSocketInfo manifest.apiSocket) ];
        })
      ]
      ++ lib.optional vm.observability.enable (node name ({
        id = "vsock-relay";
        role = "vsock-relay";
        unit = "nixling-otel-relay@${name}.service";
        readiness = [ (unixSocketExists (vsockSocketForPort manifest.observability.vsockHostSocket obsOtlpPort)) ];
      } // vsockRelayRunner name manifest))
      ++ lib.optional guestSshEnabled (node name {
        id = "guest-ssh-readiness";
        role = "guest-ssh-readiness";
        readiness = [ (tcpPort manifest.staticIp 22) ];
      })
      ++ lib.optional usbipEnabled (node name ({
        id = "usbip";
        role = "usbip";
        unit = "nixling-sys-${manifest.env}-usbipd-proxy.service";
        readiness = [ (tcpPort manifest.usbipdHostIp 3240) ];
      } // usbipRunner manifest));
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
      ++ lib.optionals vm.graphics.enable (
        (edgesFromNodes optionalSidecarBaseNodeIds "gpu" "The GPU sidecar starts only after every prerequisite sidecar is ready.")
        ++ lib.optional vm.graphics.videoSidecar
          (edge "gpu" "video" "The optional video decoder sidecar depends on the GPU sidecar.")
      )
      ++ lib.optionals vm.audio.enable (
        edgesFromNodes optionalSidecarBaseNodeIds "audio" "The audio sidecar starts only after every prerequisite sidecar is ready."
      )
      ++ edgesFromNodes preVmmNodeIds "cloud-hypervisor" "Cloud Hypervisor starts only after every prerequisite sidecar is ready."
      ++ lib.optional guestSshEnabled
        (edge "cloud-hypervisor" "guest-ssh-readiness" "SSH readiness is checked only after Cloud Hypervisor is running.")
      ++ lib.optional usbipEnabled
        (edge "guest-ssh-readiness" "usbip" "USBIP attach flows require both guest SSH readiness and the per-env proxy.");
      invariants = {
        perVmAuditPipeline = true;
        swtpmPreStartFlush = true;
        tpmOwnershipMigrationWithoutRunningVmMutation = true;
        usbipGating = true;
      };
    };

  data = {
    schemaVersion = "v2";
    vms = lib.mapAttrsToList vmDag enabledVms;
  };

  jsonText = builtins.toJSON data;
  jsonFile = pkgs.writeText "nixling-processes.json" jsonText;
in
{
  options.nixling._bundle.processesJson = lib.mkOption {
    type = lib.types.unspecified;
    readOnly = true;
    internal = true;
    description = "Internal W1 schema-v1 processes.json artifact metadata.";
  };

  config = {
    nixling._bundle.processesJson = {
      inherit data jsonText;
      path = "${jsonFile}";
    };
    environment.etc."nixling/processes.json" = privateEtc jsonFile;
  };
}
