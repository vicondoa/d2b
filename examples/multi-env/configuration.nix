{ pkgs, ... }:

let
  guestSystem = pkgs.runCommand "d2b-example-guest-system" { } ''
    mkdir -p "$out"
  '';
  cloudHypervisorProvider = pkgs.runCommand "d2b-example-cloud-hypervisor-provider" { } ''
    mkdir -p "$out"
  '';

  guest = providerName: {
    type = "Guest";
    spec = {
      providerRef = "Provider/${providerName}";
      systemArtifactId = "guest-system";
    };
  };

  runtimeProvider = {
    type = "Provider";
    spec = {
      artifactId = "cloud-hypervisor-provider";
      config.controllerExecutionRef = "Host/host";
    };
  };
in
{
  boot.loader.grub.enable = false;
  boot.loader.systemd-boot.enable = false;
  boot.initrd.includeDefaultModules = false;
  fileSystems."/" = {
    device = "tmpfs";
    fsType = "tmpfs";
  };
  environment.etc."machine-id".text = "00000000000000000000000000000000";
  networking.hostName = "demo";
  system.stateVersion = "25.11";

  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
  };

  d2b.site = {
    waylandUser = null;
    launcherUsers = [ "alice" ];
    yubikey.enable = false;
  };

  d2b.artifacts = {
    guest-system = {
      package = guestSystem;
      type = "nixos-system";
    };
    cloud-hypervisor-provider = {
      package = cloudHypervisorProvider;
      type = "provider";
    };
  };

  d2b.zones = {
    local-root.resources = {
      host = {
        type = "Host";
        spec.providerRef = "Provider/system-core";
      };
      runtime-cloud-hypervisor = runtimeProvider;
    };
    work = {
      parentZone = "local-root";
      resources = {
        host = {
          type = "Host";
          spec.providerRef = "Provider/system-core";
        };
        runtime-cloud-hypervisor = runtimeProvider;
        work-app = guest "runtime-cloud-hypervisor";
      };
    };
    personal = {
      parentZone = "local-root";
      resources = {
        host = {
          type = "Host";
          spec.providerRef = "Provider/system-core";
        };
        runtime-cloud-hypervisor = runtimeProvider;
        personal-app = guest "runtime-cloud-hypervisor";
      };
    };
  };

  d2b.guestSystems.work.work-app = {
    config.system.build.toplevel = guestSystem;
  };
  d2b.guestSystems.personal.personal-app = {
    config.system.build.toplevel = guestSystem;
  };
}
