{ config, lib, pkgs, ... }:

let
  clean = builtins.unsafeDiscardStringContext;

  cfg = config.nixling;
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

  repeatedFlagArgs = flag: params:
    lib.concatMap (param: [ flag param ]) params;

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
      flush_sock=/run/swtpm/${name}/flush.sock

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
      microvm = config.microvm.vms.${name}.config.config.microvm;
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
      vsockPath = if userVSockPath != null then userVSockPath else "notify.vsock";
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
            path = toString volume.image;
            direct = if volume.direct then "on" else "off";
            readonly = if volume.readOnly then "on" else "off";
            image_type = toString volume.imageType;
          }
          // lib.optionalAttrs (volume.serial != null) {
            serial = volume.serial;
          }
          // diskMqOps)
        ) microvm.volumes
        ++ lib.optionals (microvm.writableStoreOverlay != null) [
          (opsMapped {
            path = "${toString microvm.writableStoreOverlay}/upper";
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
        if device.bus == "pci" then
          "path=/sys/bus/pci/devices/${device.path}"
        else
          throw "Unsupported device bus ${device.bus} for cloud-hypervisor argv emission"
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
    ++ repeatedFlagArgs "--disk" diskParams
    ++ repeatedFlagArgs "--fs" fsParams
    ++ [ "--api-socket" manifest.apiSocket ]
    ++ repeatedFlagArgs "--net" netParams
    ++ repeatedFlagArgs "--device" deviceParams
    ++ processedExtraArgs
    ++ audioExtraArgs;

  virtiofsdRunner = name: share:
    let
      microvm = config.microvm.vms.${name}.config.config.microvm;
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
        "--posix-acl"
        "--xattr"
        "--cache=${share.cache or "auto"}"
      ]
      ++ lib.optionals (microvm.virtiofsd.inodeFileHandles != null) [ "--inode-file-handles=${microvm.virtiofsd.inodeFileHandles}" ]
      ++ lib.optionals (microvm.hypervisor == "crosvm") [ "--tag=${share.tag}" ]
      ++ lib.optionals (share.readOnly or false) [ "--readonly" ]
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
      "type=unixio,path=/run/swtpm/${name}/sock,mode=0600"
      "--tpm2"
      "--flags"
      "startup-clear"
    ];
  };

  gpuRunner = name:
    let
      microvm = config.microvm.vms.${name}.config.config.microvm;
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

  node = name: { id, role, readiness, unit ? null, binaryPath ? null, argv ? [ ] }:
    let
      vm = cfg.vms.${name};
      emitUnit = unit != null && vm.supervisor == "systemd";
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
    };

  edge = from: to: reason: { inherit from to reason; };
  edgesFromNodes = fromNodes: to: reason:
    builtins.map (from: edge from to reason) fromNodes;
  edgesToNodes = from: toNodes: reason:
    builtins.map (to: edge from to reason) toNodes;

  vmDag = name: vm:
    let
      manifest = cfg.manifest.${name};
      microvm = config.microvm.vms.${name}.config.config.microvm;
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
          (lib.optionals vm.graphics.enable [ "video" ])
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
        readiness = [ (unixSocketExists manifest.gpuSocket) ];
      } // gpuRunner name))
      ++ lib.optional vm.graphics.enable (node name ({
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
        ++ [ (edge "gpu" "video" "The optional video decoder sidecar depends on the GPU sidecar.") ]
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
