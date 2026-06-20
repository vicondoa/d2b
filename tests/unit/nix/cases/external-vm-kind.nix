# Foundational external VM runtime coverage for `runtime.kind = "qemu-media"`.
#
# This is intentionally eval-only: full QMP control and media hotplug lifecycle
# land in later runtime todos. These cases pin the option schema,
# qemu-media incompatibility assertions, the structural skip that keeps
# external media VMs out of the per-VM NixOS evaluator / store emitters, and
# the QMP-ready qemu-media runner contract.
{ mkEval, lib, flakeRoot, ... }:

let
  cleanGuest = flakeRoot + "/tests/unit/nix/eval-cases/guest-fixtures/clean-guest.nix";

  mkHost = { vmAttrs ? { }, includeIndex ? true }:
    { lib, ... }: {
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

      nixling.vms.media = {
        runtime.kind = "qemu-media";
        env = "work";
        qemuMedia = {
          source = {
            ref = "installer-usb";
            format = "iso";
          };
          removableSlots.cdrom.source = {
            ref = "tools-usb";
            format = "iso";
          };
        };
      } // lib.optionalAttrs includeIndex {
        index = 42;
      } // vmAttrs;
    };

  positive = mkEval [ (mkHost { }) ];
  positiveCfg = positive.config;
  positiveVm = positiveCfg.nixling.vms.media;
  positiveManifest = positiveCfg.nixling.manifest.media;
  positiveHostJson = positiveCfg.nixling._bundle.hostJson.data;
  positiveProcesses = positiveCfg.nixling._bundle.processesJson.data.vms;
  positiveQemuProcess = lib.findFirst (vm: vm.vm == "media") null positiveProcesses;
  positiveQemuNodeIds =
    if positiveQemuProcess == null then [ ] else map (node: node.id) positiveQemuProcess.nodes;
  positiveQemuRoles =
    if positiveQemuProcess == null then [ ] else map (node: node.role) positiveQemuProcess.nodes;
  positiveQemuRunner =
    if positiveQemuProcess == null then null else lib.findFirst (node: node.id == "qemu-media") null positiveQemuProcess.nodes;
  positiveProfileNames = lib.attrNames positiveCfg.nixling._bundle.minijailProfiles;

  failingMessages = args:
    let cfg = (mkEval [ (mkHost args) ]).config;
    in map (a: a.message) (builtins.filter (a: !a.assertion) cfg.assertions);

  hasFailure = args: needle:
    lib.any (message: lib.hasInfix needle message) (failingMessages args);

  badSource = mkEval [ (mkHost {
    vmAttrs.qemuMedia.source = {
      ref = "installer-usb";
      busid = "1-1";
    };
  }) ];

  badSlot = mkEval [ (mkHost {
    vmAttrs.qemuMedia.removableSlots.cdrom = {
      source = { ref = "tools-usb"; };
      busIds = [ "1-1" ];
    };
  }) ];

  badSlotSource = mkEval [ (mkHost {
    vmAttrs.qemuMedia.removableSlots.cdrom.source = {
      ref = "tools-usb";
      busId = "1-1";
    };
  }) ];

  badSourceBusids = mkEval [ (mkHost {
    vmAttrs.qemuMedia.source = {
      ref = "installer-usb";
      busids = [ "1-1" ];
    };
  }) ];

  badQemuMediaBusid = mkEval [ (mkHost {
    vmAttrs.qemuMedia = {
      busid = "1-1";
    };
  }) ];

  duplicateRefMessages = failingMessages {
    vmAttrs.qemuMedia = {
      source = {
        ref = "installer-usb";
        format = "iso";
      };
      removableSlots.cdrom.source = {
        ref = "installer-usb";
        format = "iso";
      };
    };
  };

  imageDirect = mkEval [ (mkHost {
    vmAttrs.qemuMedia = {
      source = {
        kind = "image-file";
        path = "/var/lib/nixling/images/installer.img";
        format = "raw";
        readOnly = false;
      };
      removableSlots = { };
    };
  }) ];
  imageDirectVm = imageDirect.config.nixling.vms.media;
  imageDirectHostJson = imageDirect.config.nixling._bundle.hostJson.data;
  imageDirectSource = lib.findFirst
    (source: source.vm == "media" && source.slot == "boot")
    null
    imageDirectHostJson.qemuMedia.sources;

  imageMissingPathMessages = failingMessages {
    vmAttrs.qemuMedia = {
      source = {
        kind = "image-file";
        format = "raw";
      };
      removableSlots = { };
    };
  };

  imageQcow2Messages = failingMessages {
    vmAttrs.qemuMedia = {
      source = {
        kind = "image-file";
        path = "/var/lib/nixling/images/installer.qcow2";
        format = "qcow2";
      };
      removableSlots = { };
    };
  };

  physicalPathMessages = failingMessages {
    vmAttrs.qemuMedia.source = {
      kind = "physical-usb";
      ref = "installer-usb";
      path = "/var/lib/nixling/images/not-usb.img";
    };
  };

  badImageRelativePath = mkEval [ (mkHost {
    vmAttrs.qemuMedia = {
      source = {
        kind = "image-file";
        path = "relative.img";
        format = "raw";
      };
      removableSlots = { };
    };
  }) ];

  explicitManualOnly = mkEval [ (mkHost {
    vmAttrs.autostart = false;
  }) ];

  rawArtifactText = builtins.toJSON {
    host = positiveHostJson;
    manifest = positiveManifest;
    processes = positiveQemuProcess;
  };
in
{
  "external-vm-kind/default-runtime-kind" = {
    expr = positiveCfg.nixling.vms."sys-work-net".runtime.kind;
    expected = "nixos";
  };

  "external-vm-kind/qemu-media-source-schema" = {
    expr = {
      inherit (positiveVm.qemuMedia.source) kind ref path format readOnly;
      slot = {
        inherit (positiveVm.qemuMedia.removableSlots.cdrom.source) ref path format readOnly;
      };
    };
    expected = {
      kind = "physical-usb";
      ref = "installer-usb";
      path = null;
      format = "iso";
      readOnly = true;
      slot = {
        ref = "tools-usb";
        path = null;
        format = "iso";
        readOnly = true;
      };
    };
  };

  "external-vm-kind/qemu-media-image-file-direct-config" = {
    expr = {
      inherit (imageDirectVm.qemuMedia.source) kind ref path format readOnly;
      host = {
        inherit (imageDirectSource) vm slot sourceKind format readOnly imagePath registryScope;
        mediaRefIsGenerated = lib.hasPrefix "image-" imageDirectSource.mediaRef;
      };
    };
    expected = {
      kind = "image-file";
      ref = null;
      path = "/var/lib/nixling/images/installer.img";
      format = "raw";
      readOnly = false;
      host = {
        vm = "media";
        slot = "boot";
        sourceKind = "image-file";
        format = "raw";
        readOnly = false;
        imagePath = "/var/lib/nixling/images/installer.img";
        registryScope = "direct-config-path";
        mediaRefIsGenerated = true;
      };
    };
  };

  "external-vm-kind/no-physical-usb-raw-identities-in-store-artifacts" = {
    expr =
      !(lib.hasInfix "/dev/disk/by-id" rawArtifactText)
      && !(lib.hasInfix "usb-Vendor_SecretSerial" rawArtifactText)
      && !(lib.hasInfix "1-2.3" rawArtifactText);
    expected = true;
  };

  "external-vm-kind/qemu-media-manifest-runtime" = {
    expr = positiveManifest.runtime;
    expected = {
      kind = "qemu-media";
      provider = {
        id = "local-qemu-media";
        type = "local";
        driver = "qemu";
      };
      capabilities = {
        lifecycle = true;
        display = true;
        usbHotplug = true;
        guestControl = false;
        exec = false;
        configSync = false;
        ssh = false;
        storeSync = false;
        keys = false;
        inGuestObservability = false;
      };
    };
  };

  "external-vm-kind/qemu-media-manifest-provider-neutral" = {
    expr = {
      inherit (positiveManifest) env stateDir tap bridge staticIp netVm;
    };
    expected = {
      env = "work";
      stateDir = "/var/lib/nixling/vms/media";
      tap = "work-l42";
      bridge = "br-work-lan";
      staticIp = "10.20.0.42";
      netVm = "sys-work-net";
    };
  };

  "external-vm-kind/qemu-media-manifest-no-fake-managed-artifacts" = {
    expr = {
      inherit (positiveManifest)
        apiSocket gpuSocket tpmSocket audioStateFile audioService
        sshUser usbipdHostIp graphics tpm usbipYubikey audio;
      observability = positiveManifest.observability;
    };
    expected = {
      apiSocket = null;
      gpuSocket = null;
      tpmSocket = null;
      audioStateFile = null;
      audioService = null;
      sshUser = null;
      usbipdHostIp = null;
      graphics = false;
      tpm = false;
      usbipYubikey = false;
      audio = false;
      observability = {
        enabled = false;
        vsockCid = null;
        vsockHostSocket = null;
        agentSocket = null;
      };
    };
  };

  "external-vm-kind/host-json-runtime-provider-catalog" = {
    expr = positiveHostJson.runtimeProviders;
    expected = [
      positiveCfg.nixling.manifest."sys-work-net".runtime
      positiveManifest.runtime
    ];
  };

  "external-vm-kind/host-json-qemu-media-vm-runtime-row" = {
    expr = lib.findFirst (row: row.vm == "media") null positiveHostJson.vmRuntimes;
    expected = {
      vm = "media";
      runtime = positiveManifest.runtime;
      env = "work";
      stateDir = "/var/lib/nixling/vms/media";
      tap = "work-l42";
      bridge = "br-work-lan";
      staticIp = "10.20.0.42";
      netVm = "sys-work-net";
    };
  };

  "external-vm-kind/host-json-qemu-media-opaque-refs" = {
    expr = positiveHostJson.qemuMedia;
    expected = {
      registryDir = "/var/lib/nixling/media-registry";
      runtimeRulesPath = "/run/udev/rules.d/99-nixling-media-ignore.rules";
      reloadBehavior = "Broker writes root-only runtime udev rules with UDISKS_IGNORE=1 and reloads udev rules after physical USB enrollment; direct image-file paths do not use enrollment.";
      sources = [
        {
          vm = "media";
          mediaRef = "installer-usb";
          slot = "boot";
          sourceKind = "physical-usb";
          format = "iso";
          readOnly = true;
          registryScope = "root-only-runtime-state";
        }
        {
          vm = "media";
          mediaRef = "tools-usb";
          slot = "cdrom";
          sourceKind = "physical-usb";
          format = "iso";
          readOnly = true;
          registryScope = "root-only-runtime-state";
        }
      ];
    };
  };

  "external-vm-kind/no-raw-media-identities-in-store-artifacts" = {
    expr =
      !(lib.hasInfix "/var/lib/nixling/media/install.iso" rawArtifactText)
      && !(lib.hasInfix "/var/lib/nixling/media/tools.iso" rawArtifactText)
      && !(lib.hasInfix "/dev/disk/by-id" rawArtifactText)
      && !(lib.hasInfix "usb-Vendor_SecretSerial" rawArtifactText)
      && !(lib.hasInfix "SecretSerial" rawArtifactText)
      && !(lib.hasInfix "1-2.3" rawArtifactText);
    expected = true;
  };

  "external-vm-kind/no-live-os-or-process-marker-sentinels-in-artifacts" = {
    expr =
      !(lib.hasInfix "ForbiddenLiveOSName" rawArtifactText)
      && !(lib.hasInfix "Windows" rawArtifactText)
      && !(lib.hasInfix "macOS" rawArtifactText)
      && !(lib.hasInfix "( W" rawArtifactText)
      && !(lib.hasInfix "W3fu" rawArtifactText)
      && !(lib.hasInfix "P6" rawArtifactText);
    expected = true;
  };

  "external-vm-kind/qemu-media-skips-computed" = {
    expr = positiveCfg.nixling._computed ? media;
    expected = false;
  };

  "external-vm-kind/qemu-media-skips-closures" = {
    expr = positiveCfg.nixling._bundle.closures ? media;
    expected = false;
  };

  "external-vm-kind/qemu-media-dedicated-principal" = {
    expr = {
      user = {
        inherit (positiveCfg.users.users."nixling-media-qemu-media") isSystemUser uid group description;
      };
      group = {
        inherit (positiveCfg.users.groups."nixling-media-qemu-media") gid;
      };
    };
    expected = {
      user = {
        isSystemUser = true;
        uid = positiveCfg.users.groups."nixling-media-qemu-media".gid;
        group = "nixling-media-qemu-media";
        description = "nixling QEMU media runner for VM media";
      };
      group = {
        gid = positiveCfg.users.users."nixling-media-qemu-media".uid;
      };
    };
  };

  "external-vm-kind/qemu-media-processes-runner-node" = {
    expr = {
      vmPresent = positiveQemuProcess != null;
      nodeIds = positiveQemuNodeIds;
      runnerRole = positiveQemuRunner.role;
      runnerReadiness = positiveQemuRunner.readiness;
      runnerProfileRole = positiveQemuRunner.profile.profileId;
      runnerEnv = positiveQemuRunner.env;
      runnerArgv = positiveQemuRunner.argv;
    };
    expected = {
      vmPresent = true;
      nodeIds = [ "host-reconcile" "qemu-media" ];
      runnerRole = "qemu-media-runner";
      runnerReadiness = [
        { kind = "unix-socket-listening"; value = "/run/nixling/vms/media/qmp.sock"; }
      ];
      runnerProfileRole = "vm-media-qemu-media";
      runnerEnv = [
        "GDK_BACKEND=wayland"
        "WAYLAND_DISPLAY=wayland-0"
        "XDG_RUNTIME_DIR=/run/user/1000"
      ];
      runnerArgv = [
        "nixling-qemu-media@media"
        "-nodefaults"
        "-no-user-config"
        "-S"
        "-object"
        "memory-backend-ram,id=nlram,size=4096M,dump=off,merge=off"
        "-machine"
        "q35,accel=kvm,usb=off,memory-backend=nlram"
        "-smp"
        "2"
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
        "virtio-net-pci,netdev=nl0,mac=02:76:53:AE:57:2A"
        "-qmp"
        "unix:/run/nixling/vms/media/qmp.sock,server=on,wait=off"
        "-monitor"
        "none"
        "-serial"
        "none"
        "-parallel"
        "none"
        "-name"
        "nixling-media-qemu-media"
      ];
    };
  };

  "external-vm-kind/qemu-media-processes-no-managed-guest-substrate" = {
    expr = {
      noCloudHypervisor = !(lib.elem "cloud-hypervisor" positiveQemuNodeIds)
        && !(lib.elem "cloud-hypervisor-runner" positiveQemuRoles);
      noStoreVirtiofs = !(lib.elem "store-virtiofs-preflight" positiveQemuNodeIds)
        && !(lib.any (role: role == "virtiofsd" || role == "store-virtiofs-preflight") positiveQemuRoles);
      noGuestControl = !(lib.elem "guest-control-health" positiveQemuNodeIds)
        && !(lib.elem "guest-control-health" positiveQemuRoles);
      noMediaPathInArgv =
        !(lib.any (arg: lib.hasInfix "/var/lib/nixling/media" arg) positiveQemuRunner.argv);
      noVhostNetPathInArgv =
        !(lib.any (arg: lib.hasInfix "/dev/vhost-net" arg || lib.hasInfix "vhostfd=" arg) positiveQemuRunner.argv);
    };
    expected = {
      noCloudHypervisor = true;
      noStoreVirtiofs = true;
      noGuestControl = true;
      noMediaPathInArgv = true;
      noVhostNetPathInArgv = true;
    };
  };

  "external-vm-kind/qemu-media-minijail-profile" = {
    expr = {
      hasHostReconcile = lib.elem "vm-media-host-reconcile" positiveProfileNames;
      hasQemuMedia = lib.elem "vm-media-qemu-media" positiveProfileNames;
      role = positiveCfg.nixling._bundle.minijailProfiles."vm-media-qemu-media".data.role;
      principal = positiveCfg.nixling._bundle.minijailProfiles."vm-media-qemu-media".data.principal;
      capabilities = positiveCfg.nixling._bundle.minijailProfiles."vm-media-qemu-media".data.capabilities;
      namespaces = positiveCfg.nixling._bundle.minijailProfiles."vm-media-qemu-media".data.namespaces;
      seccompPolicyRef = positiveCfg.nixling._bundle.minijailProfiles."vm-media-qemu-media".data.seccompPolicyRef;
      mountPolicy = {
        readOnlyPaths = positiveCfg.nixling._bundle.minijailProfiles."vm-media-qemu-media".data.mountPolicy.readOnlyPaths;
        writablePaths = map (p: p.path) positiveCfg.nixling._bundle.minijailProfiles."vm-media-qemu-media".data.mountPolicy.writablePaths;
        inherit (positiveCfg.nixling._bundle.minijailProfiles."vm-media-qemu-media".data.mountPolicy)
          nixStoreReadOnly hideDeviceNodesByDefault deviceBinds bindMounts;
      };
      forbiddenCaps =
        lib.any (cap: lib.elem cap positiveCfg.nixling._bundle.minijailProfiles."vm-media-qemu-media".data.capabilities)
          [ "CAP_SYS_ADMIN" "CAP_SYS_RAWIO" "CAP_DAC_OVERRIDE" "CAP_NET_ADMIN" ];
      forbiddenDeviceBinds =
        lib.any (dev: lib.elem dev positiveCfg.nixling._bundle.minijailProfiles."vm-media-qemu-media".data.mountPolicy.deviceBinds)
          [ "/dev/bus/usb" "/dev/net/tun" "/dev/vhost-net" ];
      mediaPathBinds =
        lib.any (bm:
          lib.hasPrefix "/var/lib/nixling/media" bm.src
          || lib.hasPrefix "/var/lib/nixling/media" bm.dst)
          positiveCfg.nixling._bundle.minijailProfiles."vm-media-qemu-media".data.mountPolicy.bindMounts;
    };
    expected = {
      hasHostReconcile = true;
      hasQemuMedia = true;
      role = "qemu-media-runner";
      principal = "nixling-media-qemu-media";
      capabilities = [ ];
      namespaces = {
        ipc = true;
        mount = true;
        net = false;
        pid = true;
        user = false;
        uts = false;
      };
      seccompPolicyRef = "w1-qemu-media";
      mountPolicy = {
        readOnlyPaths = [ "/" ];
        writablePaths = [
          "/run/nixling/vms/media"
          "/var/lib/nixling/vms/media"
        ];
        nixStoreReadOnly = true;
        hideDeviceNodesByDefault = true;
        deviceBinds = [ "/dev/kvm" ];
        bindMounts = [ ];
      };
      forbiddenCaps = false;
      forbiddenDeviceBinds = false;
      mediaPathBinds = false;
    };
  };

  "external-vm-kind/source-rejects-busid" = {
    expr = badSource.config.nixling.vms.media.qemuMedia.source.ref;
    expectedError = { };
  };

  "external-vm-kind/slot-rejects-busIds" = {
    expr = badSlot.config.nixling.vms.media.qemuMedia.removableSlots.cdrom.source.ref;
    expectedError = { };
  };

  "external-vm-kind/slot-source-rejects-busId" = {
    expr = badSlotSource.config.nixling.vms.media.qemuMedia.removableSlots.cdrom.source.ref;
    expectedError = { };
  };

  "external-vm-kind/source-rejects-busids" = {
    expr = badSourceBusids.config.nixling.vms.media.qemuMedia.source.ref;
    expectedError = { };
  };

  "external-vm-kind/qemuMedia-rejects-busid" = {
    expr = badQemuMediaBusid.config.nixling.vms.media.qemuMedia.source.ref;
    expectedError = { };
  };

  "external-vm-kind/rejects-duplicate-media-refs" = {
    expr = lib.any (message: lib.hasInfix "duplicate opaque ref(s): installer-usb" message) duplicateRefMessages;
    expected = true;
  };

  "external-vm-kind/rejects-image-file-without-path" = {
    expr = lib.any (message: lib.hasInfix "kind = \"image-file\" requires an absolute" message) imageMissingPathMessages;
    expected = true;
  };

  "external-vm-kind/rejects-image-file-non-raw-format" = {
    expr = lib.any (message: lib.hasInfix "supports only" message) imageQcow2Messages;
    expected = true;
  };

  "external-vm-kind/rejects-physical-usb-path" = {
    expr = lib.any (message: lib.hasInfix "kind = \"physical-usb\" must not set `path`" message) physicalPathMessages;
    expected = true;
  };

  "external-vm-kind/source-rejects-relative-image-path" = {
    expr = badImageRelativePath.config.nixling.vms.media.qemuMedia.source.path;
    expectedError = { };
  };

  "external-vm-kind/requires-env" = {
    expr = hasFailure { vmAttrs.env = lib.mkForce null; } "requires\n`env`";
    expected = true;
  };

  "external-vm-kind/requires-explicit-index" = {
    expr = hasFailure { includeIndex = false; } "requires an\nexplicit `index`";
    expected = true;
  };

  "external-vm-kind/rejects-guest-config" = {
    expr = hasFailure { vmAttrs.config.networking.hostName = "media"; } "must not define\n`config`";
    expected = true;
  };

  "external-vm-kind/rejects-guestConfigFile" = {
    expr = hasFailure { vmAttrs.guestConfigFile = cleanGuest; } "incompatible\nwith guestConfigFile";
    expected = true;
  };

  "external-vm-kind/rejects-guest-control" = {
    expr = hasFailure { vmAttrs.guest.control.enable = true; } "guest-control and guest exec";
    expected = true;
  };

  "external-vm-kind/rejects-ssh-sudo-keys" = {
    expr = hasFailure {
      vmAttrs = {
        ssh.user = "alice";
        sudo = true;
        userAuthorizedKeys = [ "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFakeKeyForEvalOnly" ];
      };
    } "nixling-managed SSH";
    expected = true;
  };

  "external-vm-kind/rejects-home-manager" = {
    expr = hasFailure { vmAttrs.homeManager.users.alice = { home.stateVersion = "25.11"; }; } "home-manager";
    expected = true;
  };

  "external-vm-kind/rejects-audit" = {
    expr = hasFailure { vmAttrs.audit.enable = true; } "guest audit";
    expected = true;
  };

  "external-vm-kind/rejects-observability" = {
    expr = hasFailure { vmAttrs.observability.enable = true; } "guest observability";
    expected = true;
  };

  "external-vm-kind/rejects-usbip" = {
    expr = hasFailure { vmAttrs.usbip.yubikey = true; } "USBIP";
    expected = true;
  };

  "external-vm-kind/rejects-graphics" = {
    expr = hasFailure { vmAttrs.graphics.enable = true; } "nixling graphics";
    expected = true;
  };

  "external-vm-kind/rejects-tpm" = {
    expr = hasFailure { vmAttrs.tpm.enable = true; } "TPM";
    expected = true;
  };

  "external-vm-kind/rejects-audio" = {
    expr = hasFailure { vmAttrs.audio.enable = true; } "audio sidecar";
    expected = true;
  };

  "external-vm-kind/rejects-autostart" = {
    expr = hasFailure { vmAttrs.autostart = true; } "manual-only";
    expected = true;
  };

  "external-vm-kind/accepts-unset-or-false-autostart" = {
    expr = {
      unset = positiveVm.autostart;
      explicitFalse = explicitManualOnly.config.nixling.vms.media.autostart;
      unsetAssertionsGreen = lib.all (a: a.assertion) positiveCfg.assertions;
      falseAssertionsGreen = lib.all (a: a.assertion) explicitManualOnly.config.assertions;
    };
    expected = {
      unset = false;
      explicitFalse = false;
      unsetAssertionsGreen = true;
      falseAssertionsGreen = true;
    };
  };
}
