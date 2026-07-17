# Host-side NixOS configuration for the d2b + entrablau
# composition example. This file owns everything *outside* the
# Entra VM: the human user on the host, the d2b site-level
# knobs, and the realm that owns the workload. The workload's NixOS
# config (and its `entrablau.*` settings) live in `work-entra.nix`.
{ lib, ... }:

{
  # Filesystem + bootloader stubs so `nixosSystem` evaluates without
  # a real `hardware-configuration.nix`. Real deployments replace
  # these with their actual disk layout.
  boot.loader.systemd-boot.enable = lib.mkDefault false;
  boot.loader.grub.enable = lib.mkDefault false;
  fileSystems."/" = lib.mkDefault {
    device = "tmpfs";
    fsType = "tmpfs";
  };
  environment.etc."machine-id".text = lib.mkDefault
    "00000000000000000000000000000000";

  # Host-side primary user. This is the human at the Plasma /
  # Wayland session — `alice` here matches the documentation
  # placeholder used throughout the d2b README. Replace with
  # your actual login name in real deployments.
  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
    extraGroups = [ "wheel" "video" "audio" ];
  };

  d2b.site = {
    # Required when any VM enables `graphics.enable` or
    # `audio.enable`. The work-entra in this example is headless
    # (TPM only, no Wayland forward), so this is informational —
    # but a realistic Entra workspace will probably want graphics
    # too, in which case waylandUser is mandatory.
    waylandUser = "alice";

    # Members of `d2b` can run `d2b vm start/stop/...`
    # through the daemon public socket. The framework adds the group;
    # you still declare the user above.
    launcherUsers = [ "alice" ];

    # Most Entra-joined workstations want a Yubikey for the MFA
    # prompt during Conditional Access flows. Leave true to keep
    # the host-side Yubico udev rules enabled; `usbip-host` then
    # loads only when an enabled VM also sets `usbip.yubikey = true`.
    # Flip false if you don't have a Yubikey.
    yubikey.enable = true;
  };
  d2b.acceptDestructiveV2Cutover = true;

  d2b.realms.work = {
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
    providers.runtime = {
      type = "runtime";
      implementationId = "cloud-hypervisor";
    };
    providers.devices = {
      type = "device";
      implementationId = "host-mediated";
    };
    providers.network = {
      type = "network";
      implementationId = "local-realm";
    };
    providers.storage = {
      type = "storage";
      implementationId = "local";
    };
  };

  # Tell d2b about the host's own physical LAN so the
  # per-env firewall blocks workload VMs from reaching the
  # host's neighbours (NAS, printer, other workstations).
  d2b.hostLanCidrs = [ "192.168.1.0/24" ];

  system.stateVersion = "25.11";
}
