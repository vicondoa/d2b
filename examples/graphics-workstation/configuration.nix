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
  # (`nixling-<vm>-snd`) speaks vhost-user-sound on the VM side and
  # pipes the stream to the host's PipeWire daemon as a regular
  # client (visible in plasma-pa as `nixling-<vm>`), so PipeWire
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
  # The Wayland-session user `nixling.site.waylandUser` references.
  # Required: the GPU + audio sidecars bind this user's
  #   /run/user/<uid>/wayland-0
  #   /run/user/<uid>/pipewire-0
  # sockets into their private mount namespaces. The framework
  # does NOT create the user — you do, here.
  # ---------------------------------------------------------------
  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
    extraGroups = [ "wheel" "video" "audio" "input" ];
    # In a real setup, drop your SSH pubkey + a hashedPassword
    # here. The example leaves them empty.
  };

  # ---------------------------------------------------------------
  # nixling.site — host-wide knobs
  # ---------------------------------------------------------------
  nixling.site = {
    # Required for any VM with graphics.enable or audio.enable.
    waylandUser = "alice";

    # Members of `nixling-launcher` get a polkit grant on the
    # framework's own systemd units; without this, `nixling up`
    # would prompt for sudo every time.
    launcherUsers = [ "alice" ];

    # Install host-side YubiKey support (udev rules for Yubico's
    # vendor ID + the `usbip-host` kernel module). Required for
    # any VM that sets `usbip.yubikey = true`. Flip to `false` on
    # hosts that don't use a YubiKey.
    yubikey.enable = true;
  };

  # Declare the host's primary LAN so the eval-time CIDR-overlap
  # check catches collisions between env subnets and the wire the
  # host actually sits on. Replace with your real LAN.
  nixling.hostLanCidrs = [ "192.168.1.0/24" ];

  # ---------------------------------------------------------------
  # nixling.envs.desktop — one isolated env
  # ---------------------------------------------------------------
  #   - lanSubnet must be /24; workload VM gets 10.42.0.10 (.index)
  #     and the auto-declared net VM takes .1.
  #   - uplinkSubnet must be /30 (point-to-point host ↔ net VM);
  #     192.0.2.0/30 is inside RFC 5737 documentation space.
  #   - env name must be ≤ 8 chars (IFNAMSIZ limit for the
  #     `br-<env>-lan` / `br-<env>-up` bridges).
  # ---------------------------------------------------------------
  nixling.envs.desktop = {
    lanSubnet    = "10.42.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };

  # ---------------------------------------------------------------
  # nixling.vms.corp-desktop — the workstation VM, full stack
  # ---------------------------------------------------------------
  # graphics + audio + YubiKey USBIP. This is the example's reason
  # for existing — the headless baseline is in
  # `examples/minimal/`; this layers all three component toggles
  # on top.
  #
  # `autostart` is intentionally left at the default `false` —
  # graphics VMs cannot autostart because there is no Wayland
  # session at multi-user.target. Use `nixling up corp-desktop`
  # from a Plasma terminal once you're logged in.
  # ---------------------------------------------------------------
  nixling.vms.corp-desktop = {
    enable = true;

    env    = "desktop";
    index  = 10;                    # IP = 10.42.0.10

    ssh.user = "alice";
    # ssh.keyPath left null: the framework-managed key under
    # /var/lib/nixling/keys/corp-desktop_ed25519 is used.

    # --- component toggles (the point of this example) ---------
    graphics.enable = true;         # crosvm GPU sidecar + Wayland cross-domain
    audio.enable    = true;         # vhost-user-sound → host PipeWire
    usbip.yubikey   = true;         # `nixling usb corp-desktop` attaches a YubiKey

    # Audio grants are OFF by default. The host-side audio sidecar
    # is installed, but the per-VM state file at
    # /var/lib/nixling/vms/corp-desktop/state/audio-state.json
    # records `mic = false, speaker = false` on first
    # materialisation. Use `nixling audio mic on corp-desktop` /
    # `nixling audio speaker on corp-desktop` to grant interactively
    # after the VM is up. The README documents the full flow.
    #
    # If you want the VM to come up with mic/speaker already
    # granted (e.g. a video-call VM), uncomment:
    #   audio.allowMicByDefault     = true;
    #   audio.allowSpeakerByDefault = true;

    config = { lib, pkgs, ... }: {
      networking.hostName = lib.mkDefault "corp-desktop";

      # The in-VM user that nixling SSHes in as. The framework
      # injects the authorized pubkey at boot via
      # `nixling-load-host-keys.service`; you only have to declare
      # the user.
      users.users.alice = {
        isNormalUser = true;
        uid = 1000;
        extraGroups = [ "wheel" ];
      };

      # Real desktop apps go here. The list below is illustrative;
      # the example leaves it empty so the eval is fast.
      # environment.systemPackages = with pkgs; [ firefox foot ];

      system.stateVersion = "25.11";
    };
  };
}
