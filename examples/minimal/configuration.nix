{ lib, ... }:

{
  # --------------------------------------------------------------
  # Host NixOS baseline — PLACEHOLDER
  # --------------------------------------------------------------
  # The values below are stubs that let `nix flake check`
  # evaluate without touching real hardware. When you copy this
  # example to a live host, replace them with your actual
  # bootloader, hardware-configuration.nix, and disk layout.
  boot.loader.systemd-boot.enable = false;
  boot.loader.grub.enable = false;
  boot.initrd.includeDefaultModules = false;
  fileSystems."/" = {
    device = "tmpfs";
    fsType = "tmpfs";
  };
  environment.etc."machine-id".text =
    "00000000000000000000000000000000";

  networking.hostName = "demo";
  system.stateVersion = "25.11";
  d2b.acceptDestructiveV2Cutover = true;

  # The single host-side user this example references. They are
  # the SSH principal into the workload VM below.
  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
    extraGroups = [ "wheel" ];
  };

  # --------------------------------------------------------------
  # d2b.site — host-wide knobs
  # --------------------------------------------------------------
  d2b.site = {
    # Headless host: no Wayland session, no graphics or audio
    # forwarding. Any VM that sets `graphics.enable = true` or
    # `audio.enable = true` will fail eval with a clear assertion
    # while this stays null.
    waylandUser = null;

    # No launcher users: no host user is added to the d2b lifecycle
    # group. sudo + the per-VM SSH key flow still cover every CLI path.
    launcherUsers = [ ];

    # No YubiKey hardware on this host. Skips the Yubico udev rules
    # and any host-side `usbip-host` loading; the per-VM
    # `usbip.yubikey` toggle (off below) is independent.
    yubikey.enable = false;
  };

  d2b.realms.personal = {
    path = "personal";
    placement = "host-local";
    broker = {
      enable = true;
      hostMutation = true;
    };
    network = {
      mode = "declared";
      lanSubnet = "10.99.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    providers.runtime = {
      type = "runtime";
      implementationId = "cloud-hypervisor";
    };
    workloads.personal-dev = {
      providerRefs.runtime = "runtime";
      config = {
        networking.hostName = lib.mkDefault "personal-dev";
        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };
      };
    };
  };
}
