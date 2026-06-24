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

  # The single host-side user this example references. They are
  # the SSH principal into the workload VM below.
  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
    extraGroups = [ "wheel" ];
  };

  # --------------------------------------------------------------
  # nixling.site — host-wide knobs
  # --------------------------------------------------------------
  nixling.site = {
    # Headless host: no Wayland session, no graphics or audio
    # forwarding. Any VM that sets `graphics.enable = true` or
    # `audio.enable = true` will fail eval with a clear assertion
    # while this stays null.
    waylandUser = null;

    # No launcher users: no host user is added to the nixling lifecycle
    # group. sudo + the per-VM SSH key flow still cover every CLI path.
    launcherUsers = [ ];

    # No YubiKey hardware on this host. Skips the Yubico udev rules
    # and any host-side `usbip-host` loading; the per-VM
    # `usbip.yubikey` toggle (off below) is independent.
    yubikey.enable = false;
  };

  # --------------------------------------------------------------
  # nixling.envs.personal — one isolated environment
  # --------------------------------------------------------------
  # Declaring this attribute set is enough for nixling to render
  # the full per-env plumbing — see this example's README for the
  # itemised list of bridges, VMs, and services that materialise.
  nixling.envs.personal = {
    lanSubnet    = "10.99.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };

  # --------------------------------------------------------------
  # nixling.vms.personal-dev — one headless workload VM
  # --------------------------------------------------------------
  # No `graphics.enable`, `audio.enable`, `tpm.enable`, or
  # `usbip.yubikey`. This is the bare-minimum nixling consumer:
  # plain DHCP guest networking, framework-managed SSH key, and
  # nothing else. Layer components on from the
  # `graphics-workstation` example next.
  nixling.vms.personal-dev = {
    enable = true;

    # Bind to the env declared above. Together with `index`, this
    # derives the VM's MAC, IP (10.99.0.10), dnsmasq reservation,
    # and tap name — no imperative wiring required.
    env   = "personal";
    index = 10;

    # `nixling switch personal-dev --apply` will SSH in as this user
    # using the framework-managed Ed25519 key generated under
    # /var/lib/nixling/keys/ on every activation.
    ssh.user = "alice";

    # NixOS module merged INTO THE GUEST (not the host). Keep
    # this minimal — the framework already handles networking,
    # SSH keys, and the per-VM /nix/store.
    config = {
      networking.hostName = lib.mkDefault "personal-dev";

      users.users.alice = {
        isNormalUser = true;
        uid = 1000;
      };
    };
  };
}
