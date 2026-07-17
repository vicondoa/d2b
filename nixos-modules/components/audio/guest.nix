# Guest support for a realm workload's mediated virtio-snd device.
#
# The realm controller owns the vhost-user process and Cloud Hypervisor
# attachment. This module configures only the guest audio stack.
#
# Architecture
# ------------
# Cloud-hypervisor has no native virtio-snd. We use its
# `--generic-vhost-user` flag (see cloud-hypervisor/docs/generic-
# vhost-user.md) to attach a vhost-user backend. The backend is
# upstream `vhost-device-sound --backend pipewire`, which connects to
# the mediated host PipeWire daemon and appears as a client named from
# the canonical workload ID, giving the user a normal per-stream mute/
# volume UX through the Plasma mixer.
#
{ lib, pkgs, config, ... }:

{
  # The workload module supplies the guest users that may open virtio-snd.
  options.d2b.audio.users = lib.mkOption {
    type = lib.types.listOf lib.types.str;
    default = [ ];
    description = ''
      Guest-side usernames to add to the `audio` group so they can
      reach the virtio-snd device + PipeWire daemon without
      logind-active session privileges.
    '';
  };

  config = {
  # Cloud-hypervisor is the only path: qemu+vhost-user-snd would
  # work too but the rest of d2b already targets CH. mkDefault so
  # graphics.nix / tpm.nix (which also pin cloud-hypervisor) don't
  # conflict.
  microvm.hypervisor = lib.mkDefault "cloud-hypervisor";

  # In-guest PipeWire + ALSA / Pulse compat stack. snd_virtio is
  # in-tree since 5.16; linuxPackages_latest on the host long
  # satisfies that.
  boot.kernelModules = [ "snd_virtio" ];

  services.pulseaudio.enable = lib.mkForce false;
  security.rtkit.enable = true;
  services.pipewire = {
    enable = true;
    alsa.enable = true;
    alsa.support32Bit = true;
    pulse.enable = true;
  };

  environment.systemPackages = with pkgs; [
    # Useful in-VM diagnostics (`pw-cli`, `pw-link`, `wpctl`).
    pipewire
    wireplumber
    alsa-utils
  ];

  # Wire the wpctl binary path into guestd so it can query and mutate the
  # workload user's PipeWire session via the AudioStatus/AudioSet RPCs.
  # The Nix store path is fixed at eval time; guestd checks the file exists
  # at startup and only advertises the capabilities when it does.
  d2b.guestControl.wpctlPath = "${pkgs.wireplumber}/bin/wpctl";

  # The virtio-snd kernel module exposes /dev/snd/{controlC0,
  # pcmC0D0c,pcmC0D0p} as root:audio mode 0660. Every interactive
  # guest user that wants to play audio needs the `audio` group; for
  # the long-lived wireplumber.service (which runs under the user's
  # systemd-user manager) this is mandatory — otherwise WP's
  # alsa-monitor silently fails to open the soundcard and `wpctl
  # status` shows empty Devices/Sinks/Sources.
  #
  # NixOS's `services.pipewire` ships a polkit rule for logind-active
  # sessions, but d2b guests don't run a graphical login manager
  # (we ssh in and the virtio-gpu compositor is part of the user's
  # home-manager session), so the polkit path is not reliably
  # triggered. Group membership is the dependable mechanism.
  #
  # We add `audio` to each listed user's extraGroups. The option is a
  # list-merge type, so this composes with whatever the workload module
  # already declares for the user.
  users.users =
    lib.listToAttrs (map
      (u: lib.nameValuePair u { extraGroups = [ "audio" ]; })
      config.d2b.audio.users);

  # WirePlumber: force "pro-audio" profile on the virtio-snd card.
  #
  # The virtio-snd ALSA driver has no ACP (Audio-Card-Profile) entry,
  # so WirePlumber's default monitor falls back to "Off" and creates
  # no Sink / Source — the card is enumerated under Devices but
  # silent.
  #
  # The card's only non-Off profile is "pro-audio" (raw multichannel
  # S32_LE 48000Hz 6ch). With that profile selected, a Sink and
  # Source are created and audio flows end-to-end:
  #
  #   Guest Firefox stereo -> guest PipeWire -> chan-mix to 6ch ->
  #   ALSA virtio-snd (S32_LE 48000Hz 6ch) -> VirtIO PCM_XFER ->
  #   vhost-device-sound sidecar -> host PipeWire 6ch stream ->
  #   chan-mix back to mono -> Plantronics playback_MONO -> speaker
  #
  # This is a lot of mixing but works in practice — verified with
  # speaker-test and Firefox WebAudio.
  #
  # Earlier we tried `use-acp = false` alone (without pinning
  # device.profile = "pro-audio") to bypass profiles entirely;
  # that left the device in "Off" mode permanently. We also tried
  # adding `api.alsa.disable-mmap = true` (because hw_params showed
  # MMAP_INTERLEAVED) — that caused speaker-test inside the guest
  # to fail with EINTR / xrun on the very first frame. Conclusion:
  # the working configuration is plain `device.profile = pro-audio`
  # + `use-acp = false`. Don't tinker with mmap settings.
  #
  # CAREFUL: `monitor.alsa.rules` is the RIGHT section for matching
  # ALSA HARDWARE cards (this is the inverse of the host-side rule
  # in audio-host.nix that uses `stream.rules` for client streams).
  services.pipewire.wireplumber.extraConfig."91-d2b-virtio-snd" = {
    "monitor.alsa.rules" = [
      {
        matches = [
          { "device.name" = "~alsa_card.pci-.*06\\.0$"; }
        ];
        actions = {
          update-props = {
            "device.profile" = "pro-audio";
            "api.alsa.use-acp" = false;
          };
        };
      }
    ];
  };
  };
}
