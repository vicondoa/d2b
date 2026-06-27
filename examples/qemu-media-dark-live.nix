{ ... }:

{
  # Safe, eval-only qemu-media example for the requested `dark` env and
  # `dark-live` VM. Physical USB devices are referenced only by opaque
  # media refs; discover live hardware at runtime with `d2b usb probe`,
  # keeping the transient probe selector on the CLI and never in this file.
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

  d2b.envs.dark = {
    lanSubnet = "10.60.0.0/24";
    uplinkSubnet = "203.0.113.0/30";
  };

  d2b.vms.dark-live = {
    enable = true;
    runtime.kind = "qemu-media";
    env = "dark";
    index = 10;
    autostart = false;

    qemuMedia = {
      source = {
        kind = "physical-usb";
        ref = "boot";
        format = "raw";
        readOnly = true;
        usbSelector.byIdName = "usb-Example_Dark_Live_0001-0:0";
      };

      removableSlots.backup.source = {
        kind = "physical-usb";
        ref = "backup";
        format = "raw";
        readOnly = true;
      };

    };

    ui.border.activeColor = "#301934";
  };
}
