# Realm-native eval-time assertion corpus.
{ flakeRoot ? null, nixpkgs ? null, d2bModule ? null }:

let
  shared = import ./shared.nix { inherit flakeRoot nixpkgs d2bModule; };

  desktopProviders = {
    display = {
      type = "display";
      implementationId = "wayland";
    };
    devices = {
      type = "device";
      implementationId = "host-mediated";
    };
  };

  desktopBindings = {
    display = "display";
    device = "devices";
  };
in
shared.mkBatch {
  cases = {
    "destructive-cutover-acknowledgement-required" = {
      expectedSubstring = "d2b.acceptDestructiveV2Cutover must be set to true";
      override = { lib, ... }: {
        d2b.acceptDestructiveV2Cutover = lib.mkForce false;
      };
    };

    "graphics-without-wayland-user" = {
      expectedSubstring = "requires d2b.site.waylandUser";
      override = { ... }: {
        d2b.site.waylandUser = null;
        d2b.realms.work.providers = desktopProviders;
        d2b.realms.work.workloads.corp-vm = {
          providerRefs = desktopBindings;
          graphics.enable = true;
        };
      };
    };

    "wayland-user-missing" = {
      expectedSubstring = "requires its d2b.site.waylandUser to name a declared host user";
      override = { lib, ... }: {
        d2b.site.waylandUser = lib.mkForce "ghost";
        d2b.realms.work.providers = desktopProviders;
        d2b.realms.work.workloads.corp-vm = {
          providerRefs = desktopBindings;
          graphics.enable = true;
        };
      };
    };

    "realm-name-invalid" = {
      expectedSubstring = "Enabled d2b.realms attribute names must be canonical labels";
      override = { ... }: {
        d2b.realms."42work".enable = true;
      };
    };

    "realm-name-reserved" = {
      expectedSubstring = "reserved target labels all or d2b";
      override = { ... }: {
        d2b.realms.all.enable = true;
      };
    };

    "workload-name-invalid" = {
      expectedSubstring = "Workload attribute names in d2b.realms.work must be canonical labels";
      override = { ... }: {
        d2b.realms.work.workloads."42desktop".providerRefs.runtime = "runtime";
      };
    };

    "workload-runtime-binding-required" = {
      expectedSubstring = "must bind providerRefs.runtime explicitly";
      override = { lib, ... }: {
        d2b.realms.work.workloads.corp-vm.providerRefs = lib.mkForce { };
      };
    };

    "workload-provider-binding-must-resolve" = {
      expectedSubstring = "selects undeclared device provider missing";
      override = { ... }: {
        d2b.realms.work.workloads.corp-vm.providerRefs.device = "missing";
      };
    };

    "graphics-requires-device-provider-binding" = {
      expectedSubstring = "require an explicit device provider binding";
      override = { ... }: {
        d2b.realms.work.providers.display = {
          type = "display";
          implementationId = "wayland";
        };
        d2b.realms.work.workloads.corp-vm = {
          providerRefs.display = "display";
          graphics.enable = true;
        };
      };
    };

    "audio-requires-audio-provider-binding" = {
      expectedSubstring = "audio requires an explicit audio provider binding";
      override = { ... }: {
        d2b.realms.work.workloads.corp-vm.audio.enable = true;
      };
    };

    "wayland-requires-display-provider-binding" = {
      expectedSubstring = "Wayland display requires an explicit display provider binding";
      override = { ... }: {
        d2b.realms.work.workloads.corp-vm.display.wayland = true;
      };
    };

    "lansubnet-wrong-mask" = {
      expectedSubstring = "network.lanSubnet must be an IPv4 /24";
      override = { lib, ... }: {
        d2b.realms.work.network.lanSubnet = lib.mkForce "10.99.0.0/23";
      };
    };

    "uplinksubnet-wrong-mask" = {
      expectedSubstring = "network.uplinkSubnet must be an IPv4 /30";
      override = { lib, ... }: {
        d2b.realms.work.network.uplinkSubnet = lib.mkForce "192.0.2.0/29";
      };
    };

    "lansubnet-nonzero-host" = {
      expectedSubstring = "network.lanSubnet must be an IPv4 /24 network ending in .0";
      override = { lib, ... }: {
        d2b.realms.work.network.lanSubnet = lib.mkForce "10.99.0.5/24";
      };
    };

    "realm-network-overlap" = {
      expectedSubstring = "d2b realm networks must use disjoint CIDRs";
      override = { ... }: {
        d2b.realms.other = {
          path = "other";
          placement = "host-local";
          broker = {
            enable = true;
            hostMutation = true;
          };
          network = {
            mode = "declared";
            lanSubnet = "10.20.0.0/24";
            uplinkSubnet = "198.51.100.0/30";
          };
        };
      };
    };

    "realm-network-host-overlap" = {
      expectedSubstring = "must be valid and disjoint from realm network";
      override = { ... }: {
        d2b.hostLanCidrs = [ "10.20.0.0/16" ];
      };
    };

    "allow-east-west-requires-site-ack" = {
      expectedSubstring = "network.lan.allowEastWest requires d2b.site.allowUnsafeEastWest";
      override = { ... }: {
        d2b.realms.work.network.lan.allowEastWest = true;
      };
    };

    "external-attachment-requires-interface" = {
      expectedSubstring = "externalNetwork.attachment.enable requires attachment.interface";
      override = { ... }: {
        d2b.realms.work.network.externalNetwork.attachment.enable = true;
      };
    };

    "external-egress-requires-attachment" = {
      expectedSubstring = "externalNetwork.egress.enable requires attachment.enable";
      override = { ... }: {
        d2b.realms.work.network.externalNetwork.egress.enable = true;
      };
    };

    "port-forward-target-must-be-local" = {
      expectedSubstring = "must select exactly one valid local workload or targetIp";
      override = { ... }: {
        d2b.realms.work.network.externalNetwork = {
          attachment = {
            enable = true;
            interface = "eno1";
          };
          portForwards = [{
            listenPort = 8443;
            workload = "missing";
            targetPort = 443;
          }];
        };
      };
    };

    "platform-gate-graphics-aarch64" = {
      expectedSubstring = "graphics/audio components are supported only on x86_64-linux";
      system = "aarch64-linux";
      override = { ... }: {
        d2b.realms.work.providers = desktopProviders;
        d2b.realms.work.workloads.corp-vm = {
          providerRefs = desktopBindings;
          graphics.enable = true;
        };
      };
    };

    "platform-gate-audio-aarch64" = {
      expectedSubstring = "graphics/audio components are supported only on x86_64-linux";
      system = "aarch64-linux";
      override = { ... }: {
        d2b.realms.work.providers.sound = {
          type = "audio";
          implementationId = "pipewire-vhost-user";
        };
        d2b.realms.work.workloads.corp-vm = {
          providerRefs.audio = "sound";
          audio.enable = true;
        };
      };
    };

    "graphics-with-autostart" = {
      expectedSubstring = "graphics/audio mediation is incompatible with autostart";
      override = { ... }: {
        d2b.realms.work.providers = desktopProviders;
        d2b.realms.work.workloads.corp-vm = {
          providerRefs = desktopBindings;
          graphics.enable = true;
          autostart = true;
        };
      };
    };

    "video-requires-graphics" = {
      expectedSubstring = "video mediation requires graphics.enable";
      override = { ... }: {
        d2b.realms.work.providers.devices = {
          type = "device";
          implementationId = "host-mediated";
        };
        d2b.realms.work.workloads.corp-vm = {
          providerRefs.device = "devices";
          graphics.videoSidecar = true;
        };
      };
    };

    "nvidia-video-requires-sidecar" = {
      expectedSubstring = "NVIDIA video decode requires videoSidecar";
      override = { ... }: {
        d2b.realms.work.providers = desktopProviders;
        d2b.realms.work.workloads.corp-vm = {
          providerRefs = desktopBindings;
          graphics = {
            enable = true;
            videoNvidiaDecode = true;
          };
        };
      };
    };

    "usbip-fido-mutual-exclusion" = {
      expectedSubstring = "cannot request USBIP and FIDO security-key mediation simultaneously";
      override = { ... }: {
        d2b.realms.work.providers.devices = {
          type = "device";
          implementationId = "host-mediated";
        };
        d2b.realms.work.workloads.corp-vm = {
          providerRefs.device = "devices";
          usbip.enable = true;
          securityKey.enable = true;
        };
      };
    };

    "device-binding-required-when-ambiguous" = {
      expectedSubstring = "multiple host-mediated providers require an explicit device provider binding";
      override = { ... }: {
        d2b.realms.work.providers = {
          devices-a = {
            type = "device";
            implementationId = "host-mediated";
          };
          devices-b = {
            type = "device";
            implementationId = "host-mediated";
          };
        };
        d2b.realms.work.workloads.corp-vm.tpm.enable = true;
      };
    };
  };
}
