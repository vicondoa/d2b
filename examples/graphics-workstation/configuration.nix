{ config, lib, pkgs, ... }:

{
  # ---------------------------------------------------------------
  # Host NixOS baseline — PLACEHOLDER
  # ---------------------------------------------------------------
  # The values below are stubs that let `nix flake check` evaluate
  # without touching real hardware. When you copy this example to a
  # live desktop, replace them with your actual bootloader, your
  # generated `hardware-configuration.nix`, and disk layout (LUKS +
  # systemd-boot is the common setup for a desktop workstation).
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

  # ---------------------------------------------------------------
  # Host Wayland session. Pick whatever compositor you actually
  # run — sway, Hyprland, GNOME, KDE Plasma — they all forward
  # Wayland surfaces the same way to the GPU sidecar. The
  # framework needs a running compositor for the Wayland user
  # named below; it does not care which compositor.
  #
  # The block is commented out so the example evaluates without
  # pulling the SDDM + Plasma closure on every `nix flake check`.
  # Uncomment when copying to a live host.
  # ---------------------------------------------------------------
  # services.displayManager.sddm = {
  #   enable = true;
  #   wayland.enable = true;
  # };
  # services.desktopManager.plasma6.enable = true;

  # ---------------------------------------------------------------
  # Host audio: PipeWire. The framework's audio sidecar
  # (`d2b-<vm>-snd`) speaks vhost-user-sound on the VM side and
  # pipes the stream to the host's PipeWire daemon as a regular
  # client (visible in plasma-pa as `d2b-<vm>`), so PipeWire
  # must be running on the host for audio.enable to do anything.
  #
  # Commented out for the same eval-cost reason as the Wayland
  # session above — uncomment on the real host.
  # ---------------------------------------------------------------
  # services.pipewire = {
  #   enable = true;
  #   alsa.enable = true;
  #   pulse.enable = true;
  # };
  # security.rtkit.enable = true;

  # ---------------------------------------------------------------
  # The Wayland-session user `d2b.site.waylandUser` references.
  # Required: the Wayland proxy (`d2b-<vm>-wlproxy`) connects
  # to this user's compositor socket; the audio sidecar also uses this
  # user's PipeWire socket.
  #   /run/user/<uid>/<waylandDisplay>   (filter proxy; not the GPU sidecar)
  #   /run/user/<uid>/pipewire-0
  # The framework does NOT create the user — you do, here.
  # ---------------------------------------------------------------
  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
    extraGroups = [ "wheel" "video" "audio" "input" ];
    # In a real setup, drop your SSH pubkey + a hashedPassword
    # here. The example leaves them empty.
  };

  # ---------------------------------------------------------------
  # d2b.site — host-wide knobs
  # ---------------------------------------------------------------
  d2b.site = {
    # Required for any VM with graphics.enable or audio.enable.
    waylandUser = "alice";

    # Members of `d2b` can call the daemon public socket for
    # lifecycle operations such as `d2b vm start`.
    launcherUsers = [ "alice" ];

    # Install host-side YubiKey support (Yubico udev rules; the
    # `usbip-host` kernel module is loaded only when an enabled VM
    # also sets `usbip.yubikey = true`). Required for any VM that
    # uses `d2b usb <vm>`. Flip to `false` on hosts without one.
    yubikey.enable = true;
  };

  # Declare the host's primary LAN so the eval-time CIDR-overlap
  # check catches collisions between env subnets and the wire the
  # host actually sits on. Replace with your real LAN.
  d2b.hostLanCidrs = [ "192.168.1.0/24" ];

  d2b.acceptDestructiveV2Cutover = true;

  d2b.realms.desktop = {
    path = "desktop.local-root";
    placement = "host-local";

    providers.runtime = {
      type = "runtime";
      implementationId = "cloud-hypervisor";
      capabilities = [ "start" "stop" "exec" ];
    };
    providers.devices = {
      type = "device";
      implementationId = "host-mediated";
      capabilities = [
        "plan-attach"
        "attach"
        "inspect"
        "adopt"
        "detach"
      ];
    };

    network = {
      mode = "declared";
      lanSubnet = "10.42.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };

    workloads.corp-desktop = {
      provider = "runtime";
      autostart = false;
      launcher = {
        enable = true;
        label = "Corporate desktop";
        capabilities = [ "gpu" "usbip" ];
      };
      config = { pkgs, ... }: {
        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
          extraGroups = [ "wheel" ];
        };
        # environment.systemPackages = with pkgs; [ firefox foot ];
        system.stateVersion = "25.11";
      };
    };
  };
}
