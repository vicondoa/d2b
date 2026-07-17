{ lib, ... }:

{
  boot.loader.grub.enable = false;
  boot.loader.systemd-boot.enable = false;
  boot.initrd.includeDefaultModules = false;
  fileSystems."/" = {
    device = "tmpfs";
    fsType = "tmpfs";
  };
  environment.etc."machine-id".text = "00000000000000000000000000000000";
  system.stateVersion = "25.11";

  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
  };

  d2b.acceptDestructiveV2Cutover = true;

  d2b.site = {
    launcherUsers = [ "alice" ];
    yubikey.enable = false;
  };

  d2b.hostLanCidrs = [ "192.168.1.0/24" ];

  d2b.realms.work = {
    id = "work";
    path = "work";
    placement = "host-local";
    broker = {
      enable = true;
      hostMutation = true;
    };
    network = {
      mode = "declared";
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    providers.vm = {
      type = "runtime";
      implementationId = "cloud-hypervisor";
    };
    workloads.work-app = {
      provider = "vm";
      config = {
        networking.hostName = lib.mkDefault "work-app";
        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };
      };
    };
  };

  d2b.realms.personal = {
    id = "personal";
    path = "personal";
    placement = "host-local";
    broker = {
      enable = true;
      hostMutation = true;
    };
    network = {
      mode = "declared";
      lanSubnet = "10.30.0.0/24";
      uplinkSubnet = "198.51.100.0/30";
    };
    providers.vm = {
      type = "runtime";
      implementationId = "cloud-hypervisor";
    };
    workloads.personal-app = {
      provider = "vm";
      config = {
        networking.hostName = lib.mkDefault "personal-app";
        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };
      };
    };
  };
}
