{ ... }:

{
  # Safe, eval-only qemu-media example for the requested `dark` env and
  # `dark-live` VM. Physical USB devices are referenced only by opaque
  # media refs; enroll live hardware at runtime with `nixling usb probe`
  # followed by `nixling usb enroll dark-live <ref> ... --apply`, keeping
  # the transient probe selector on the CLI and never in this file.
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

  nixling.site = {
    waylandUser = "alice";
    launcherUsers = [ "alice" ];
    niriVmBorders.enable = true;
    yubikey.enable = false;
  };

  nixling.hostLanCidrs = [
    "192.168.1.0/24"
  ];

  nixling.envs.dark = {
    lanSubnet = "10.60.0.0/24";
    uplinkSubnet = "203.0.113.0/30";
  };

  nixling.vms.dark-live = {
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
      };

      removableSlots.backup.source = {
        kind = "physical-usb";
        ref = "backup";
        format = "raw";
        readOnly = true;
      };

      window.niriBorderColor = "#301934";
    };
  };
}
