{ mkEval, lib, system, ... }:

let
  x86 = system == "x86_64-linux";

  fixture = { lib, ... }: {
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
      yubikey.enable = true;
      allowUnsafeEastWest = true;
    };
    nixling.hostLanCidrs = [ "172.16.0.0/12" "10.0.0.0/8" ];
    nixling.observability.enable = true;

    nixling.envs.zeta = {
      lanSubnet = "10.50.0.0/24";
      uplinkSubnet = "198.51.100.0/30";
      hostBlocklist = [ "203.0.113.0/24" "192.168.0.0/16" ];
    };
    nixling.envs.alpha = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
      lan.allowEastWest = true;
      mtu = 1280;
      mssClamp = true;
    };

    nixling.vms.zed = {
      env = "zeta";
      index = 12;
      ssh.user = "alice";
      tpm.enable = true;
      usbip = {
        yubikey = true;
        busids = [ "1-1.4" "1-1.2" ];
      };
      audio.enable = x86;
      observability.enable = true;
      guest.control.enable = true;
      guest.exec.enable = true;
      guest.shell = {
        enable = true;
        defaultName = "ops";
        maxSessions = 3;
        maxAttached = 2;
      };
      config.users.users.alice = { isNormalUser = true; uid = 1000; };
    };

    nixling.vms.app = {
      env = "alpha";
      index = 10;
      ssh.user = "alice";
      graphics = {
        enable = x86;
        videoSidecar = x86;
      };
      observability.enable = true;
      config.users.users.alice = { isNormalUser = true; uid = 1000; };
    };

    nixling.vms.media = {
      runtime.kind = "qemu-media";
      env = "alpha";
      index = 20;
      usbip.yubikey = true;
      qemuMedia = {
        source = {
          ref = "installer-usb";
          format = "iso";
        };
        removableSlots.tools.source = {
          ref = "tools-usb";
          format = "iso";
          usbSelector.byIdName = "usb-Test_Tools_0001-0:0";
        };
      };
    };

    nixling.vms.disabled = {
      enable = false;
      env = "zeta";
      index = 99;
      tpm.enable = true;
    };
  };

  cfg = (mkEval [ fixture ]).config;
  index = cfg.nixling._index;
  expectedVmNames = [
    "app"
    "media"
    "sys-alpha-net"
    "sys-obs"
    "sys-obs-net"
    "sys-zeta-net"
    "zed"
  ];
in
{
  "index/shape-and-sorting" = {
    expr = {
      enabledEnvNames = index.enabledEnvNames;
      enabledVmNames = index.enabledVmNames;
      normalNixosVmNames = index.normalNixosVmNames;
      qemuMediaVmNames = index.qemuMediaVmNames;
      netVmNames = index.netVmNames;
      workloadNamesByEnv = index.workloadNamesByEnv;
    };
    expected = {
      enabledEnvNames = [ "alpha" "obs" "zeta" ];
      enabledVmNames = expectedVmNames;
      normalNixosVmNames = [
        "app"
        "sys-alpha-net"
        "sys-obs"
        "sys-obs-net"
        "sys-zeta-net"
        "zed"
      ];
      qemuMediaVmNames = [ "media" ];
      netVmNames = [ "sys-alpha-net" "sys-obs-net" "sys-zeta-net" ];
      workloadNamesByEnv = {
        alpha = [ "app" "media" ];
        obs = [ "sys-obs" ];
        zeta = [ "zed" ];
      };
    };
  };

  "index/env-meta-compat-alias" = {
    expr = cfg.nixling._envMeta == index.envMeta;
    expected = true;
  };

  "index/allow-east-west-metadata" = {
    expr = {
      alpha = index.envMeta.alpha.allowEastWest;
      zeta = index.envMeta.zeta.allowEastWest;
      alphaBridge = index.envMeta.alpha.lanBridge;
      alphaWorkloads = index.envMeta.alpha.workloads;
    };
    expected = {
      alpha = true;
      zeta = false;
      alphaBridge = "br-alpha-lan";
      alphaWorkloads = {
        app = {
          ip = "10.20.0.10";
          mac = "02:70:C9:07:75:0A";
          hostName = "app";
        };
        media = {
          ip = "10.20.0.20";
          mac = "02:70:C9:07:75:14";
          hostName = "media";
        };
      };
    };
  };

  "index/component-and-usbip-subsets" = {
    expr = {
      graphics = index.components.graphics.vmNames;
      audio = index.components.audio.vmNames;
      video = index.components.video.vmNames;
      tpm = index.components.tpm.vmNames;
      usbip = index.components.usbip.vmNames;
      usbipEnvNames = index.usbip.envNames;
      activeUsbipEnvNames = index.usbip.activeEnvNames;
      usbipVmNamesByEnv = index.usbip.vmNamesByEnv;
      zetaBusidLocks = index.usbip.busidLocksByEnv.zeta;
      zetaHostBlocklist = index.envMeta.zeta.hostBlocklist;
    };
    expected = {
      graphics = lib.optional x86 "app";
      audio = lib.optional x86 "zed";
      video = lib.optional x86 "app";
      tpm = [ "zed" ];
      usbip = [ "media" "zed" ];
      usbipEnvNames = [ "alpha" "zeta" ];
      activeUsbipEnvNames = [ "alpha" "zeta" ];
      usbipVmNamesByEnv = {
        alpha = [ "media" ];
        obs = [ ];
        zeta = [ "zed" ];
      };
      zetaBusidLocks = [{
        vm = "zed";
        lockOwner = "daemon";
        scope = "per-busid";
        busIds = [ "1-1.2" "1-1.4" ];
      }];
      zetaHostBlocklist = [
        "10.0.0.0/8"
        "10.20.0.0/24"
        "10.40.0.0/24"
        "172.16.0.0/12"
        "192.0.2.0/30"
        "192.168.0.0/16"
        "203.0.113.0/24"
        "203.0.113.0/30"
      ];
    };
  };

  "index/observability-and-shell-subsets" = {
    expr = {
      observed = index.components.observability.vmNames;
      sourcePorts = index.observability.sourcePorts;
      relayVmNames = index.observability.relayVmNames;
      byRole = index.observability.byRole;
      backendPorts = index.observability.backendPorts;
      shellVmNames = index.guestShell.vmNames;
      shellLimits = index.guestShell.limits;
    };
    expected = {
      observed = [ "app" "zed" ];
      sourcePorts = {
        app = 14318;
        zed = 14319;
      };
      relayVmNames = [ "app" "zed" ];
      byRole = {
        host = [ "host" ];
        workload = [ "app" "zed" ];
        relay = [ "app" "zed" ];
        stack = [ "sys-obs" ];
      };
      backendPorts = {
        grafana = 3000;
        signoz = 8080;
        otlpGrpc = 4317;
        otlpHttp = 4318;
        hostRelayVsock = 14317;
      };
      shellVmNames = [ "zed" ];
      shellLimits = {
        zed = {
          enable = true;
          defaultName = "ops";
          maxSessions = 3;
          maxAttached = 2;
          controlEnabled = true;
          execEnabled = true;
        };
      };
    };
  };

  "index/qemu-media-and-runtime-subsets" = {
    expr = {
      manualOnlyVmNames = index.qemuMedia.manualOnlyVmNames;
      runtimeMediaVmNames = index.qemuMedia.runtimeMediaVmNames;
      sources = index.qemuMedia.sources;
      runtimeKinds = index.runtime.kinds;
      mediaRuntimeKind = index.runtime.byVm.media.kind;
      providerIds = map (row: row.provider.id) index.runtime.providers;
    };
    expected = {
      manualOnlyVmNames = [ "media" ];
      runtimeMediaVmNames = [ "media" ];
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
          slot = "tools";
          sourceKind = "physical-usb";
          format = "iso";
          readOnly = true;
          registryScope = "root-only-runtime-state";
          usbSelector.byIdName = "usb-Test_Tools_0001-0:0";
        }
      ];
      runtimeKinds = [ "nixos" "qemu-media" ];
      mediaRuntimeKind = "qemu-media";
      providerIds = [ "local-cloud-hypervisor" "local-qemu-media" ];
    };
  };

  "index/computed-net-vms-remain-reachable" = {
    expr =
      builtins.hasAttr "sys-alpha-net" cfg.nixling._computed
      && cfg.nixling._computed.sys-alpha-net.config.networking.hostName == "sys-alpha-net";
    expected = true;
  };
}
