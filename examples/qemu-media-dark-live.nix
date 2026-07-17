{ ... }:

{
  # Safe, eval-only realm-owned qemu-media workload.
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

  d2b.site = {
    waylandUser = "alice";
    launcherUsers = [ "alice" ];
    ui.compositors.niri.enable = true;
    yubikey.enable = false;
  };

  d2b.hostLanCidrs = [
    "192.168.1.0/24"
  ];

  d2b.realms = {
    local-root = {
    path = "local-root";
    placement = "host-local";
    };
    dark = {
    parent = "local-root";
    path = "dark.local-root";
    placement = "host-local";
    allowedUsers = [ "alice" ];
    network = {
      mode = "declared";
      lanSubnet = "10.60.0.0/24";
      uplinkSubnet = "203.0.113.0/30";
    };
    providers.media = {
      type = "runtime";
      implementationId = "qemu-media";
      configRef = "dark-live-media";
      capabilities = [ "qmp-media-attach" ];
    };
    workloads.dark-live = {
      provider = "media";
      autostart = false;
      launcher = {
        enable = true;
        label = "Dark live media";
      };
    };
    };
  };
}
