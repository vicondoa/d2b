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

    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = true;
      allowUnsafeEastWest = true;
    };
    d2b.hostLanCidrs = [ "172.16.0.0/12" "10.0.0.0/8" ];
    d2b.observability.enable = true;

    d2b.envs.zeta = {
      lanSubnet = "10.50.0.0/24";
      uplinkSubnet = "198.51.100.0/30";
      hostBlocklist = [ "203.0.113.0/24" "192.168.0.0/16" ];
    };
    d2b.envs.empty = {
      lanSubnet = "10.40.0.0/24";
      uplinkSubnet = "203.0.113.0/30";
    };
    d2b.envs.disabled = {
      enable = false;
      lanSubnet = "10.60.0.0/24";
      uplinkSubnet = "203.0.113.4/30";
    };
    d2b.envs.alpha = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
      lan.allowEastWest = true;
      mtu = 1280;
      mssClamp = true;
      externalNetwork = {
        enable = true;
        attachment = {
          enable = true;
          interface = "eno1";
        };
        egress = {
          enable = true;
          allowedCidrs = [ "192.168.1.0/24" ];
        };
        portForwards = [{
          protocol = "tcp";
          listenPort = 8443;
          vm = "app";
          targetPort = 443;
          sourceCidrs = [ "192.168.1.0/24" ];
        }];
        mdns = {
          enable = true;
          publishWorkstation = true;
        };
      };
    };

    d2b.vms.zed = {
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

    d2b.vms.app = {
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

    d2b.vms.media = {
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

    d2b.vms.disabled = {
      enable = false;
      env = "zeta";
      index = 99;
      tpm.enable = true;
    };
  };

  cfg = (mkEval [ fixture ]).config;
  cfgYubikeyDisabled = (mkEval [ fixture ({ lib, ... }: {
    d2b.site.yubikey.enable = lib.mkForce false;
  }) ]).config;
  index = cfg.d2b._index;
  expectedVmNames = [
    "app"
    "media"
    "sys-alpha-net"
    "sys-empty-net"
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
      enabledEnvNames = [ "alpha" "empty" "obs" "zeta" ];
      enabledVmNames = expectedVmNames;
      normalNixosVmNames = [
        "app"
        "sys-alpha-net"
        "sys-empty-net"
        "sys-obs"
        "sys-obs-net"
        "sys-zeta-net"
        "zed"
      ];
      qemuMediaVmNames = [ "media" ];
      netVmNames = [ "sys-alpha-net" "sys-empty-net" "sys-obs-net" "sys-zeta-net" ];
      workloadNamesByEnv = {
        alpha = [ "app" "media" ];
        empty = [ ];
        obs = [ "sys-obs" ];
        zeta = [ "zed" ];
      };
    };
  };

  "index/env-meta-compat-alias" = {
    expr = cfg.d2b._envMeta == index.envMeta;
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

  "index/guest-journal-bounded" = {
    expr = cfg.d2b._computed.zed.config.services.journald.extraConfig;
    expected = ''
      SystemMaxUse=512M
      SystemKeepFree=512M
      RuntimeMaxUse=128M
    '';
  };

  "index/home-lan-metadata" = {
    expr = {
      envNames = index.externalNetwork.envNames;
      alpha = index.envMeta.alpha.externalNetwork;
      alphaFromExternalNetworkIndex = index.externalNetwork.envMeta.alpha.externalNetwork;
    };
    expected = {
      envNames = [ "alpha" ];
      alpha = {
        enable = true;
        attachment = {
          enable = true;
          interface = "eno1";
          mode = "macvtap";
          macvtapMode = "bridge";
          macAddress = "02:4A:E9:D5:17:03";
          hostIfName = "alpha-h0";
          guestIfName = "external0";
          ipv4 = {
            method = "dhcp";
            address = null;
            gateway = null;
            dns = [ ];
          };
        };
        egress = {
          enable = true;
          allowedCidrs = [ "192.168.1.0/24" ];
          masquerade = true;
        };
        portForwards = [{
          listenPort = 8443;
          protocol = "tcp";
          vm = "app";
          sourceCidrs = [ "192.168.1.0/24" ];
          targetIp = "10.20.0.10";
          targetPort = 443;
        }];
        mdns = {
          enable = true;
          reflector.enable = true;
          dnsmasqLocal = {
            enable = false;
            port = 53530;
          };
          publishWorkstation = true;
        };
      };
      alphaFromExternalNetworkIndex = {
        enable = true;
        attachment = {
          enable = true;
          interface = "eno1";
          mode = "macvtap";
          macvtapMode = "bridge";
          macAddress = "02:4A:E9:D5:17:03";
          hostIfName = "alpha-h0";
          guestIfName = "external0";
          ipv4 = {
            method = "dhcp";
            address = null;
            gateway = null;
            dns = [ ];
          };
        };
        egress = {
          enable = true;
          allowedCidrs = [ "192.168.1.0/24" ];
          masquerade = true;
        };
        portForwards = [{
          listenPort = 8443;
          protocol = "tcp";
          vm = "app";
          sourceCidrs = [ "192.168.1.0/24" ];
          targetIp = "10.20.0.10";
          targetPort = 443;
        }];
        mdns = {
          enable = true;
          reflector.enable = true;
          dnsmasqLocal = {
            enable = false;
            port = 53530;
          };
          publishWorkstation = true;
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
      usbipBackendPorts = index.usbip.backendPorts;
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
        empty = [ ];
        obs = [ ];
        zeta = [ "zed" ];
      };
      usbipBackendPorts = {
        alpha = 3241;
        empty = 3242;
        obs = 3243;
        zeta = 3244;
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

  "index/site-yubikey-disabled-suppresses-runtime-usbip" = {
    expr = {
      declaredVmOptIns = cfgYubikeyDisabled.d2b._index.usbip.vmNames;
      declaredEnvOptIns = cfgYubikeyDisabled.d2b._index.usbip.envNames;
      activeEnvNames = cfgYubikeyDisabled.d2b._index.usbip.activeEnvNames;
      busidLocksByEnv = cfgYubikeyDisabled.d2b._index.usbip.busidLocksByEnv;
      hostJsonLocks = lib.listToAttrs (map
        (env: { name = env.env; value = env.usbipBusidLocks; })
        cfgYubikeyDisabled.d2b._bundle.hostJson.data.environments);
      hostJsonBackendPorts = lib.listToAttrs (map
        (env: { name = env.env; value = env.usbipBackendPort or null; })
        cfgYubikeyDisabled.d2b._bundle.hostJson.data.environments);
    };
    expected = {
      declaredVmOptIns = [ "media" "zed" ];
      declaredEnvOptIns = [ "alpha" "zeta" ];
      activeEnvNames = [ ];
      busidLocksByEnv = {
        alpha = [ ];
        empty = [ ];
        obs = [ ];
        zeta = [ ];
      };
      hostJsonLocks = {
        alpha = [ ];
        empty = [ ];
        obs = [ ];
        zeta = [ ];
      };
      hostJsonBackendPorts = {
        alpha = null;
        empty = null;
        obs = null;
        zeta = null;
      };
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
      builtins.hasAttr "sys-alpha-net" cfg.d2b._computed
      && cfg.d2b._computed.sys-alpha-net.config.networking.hostName == "sys-alpha-net";
    expected = true;
  };
}
