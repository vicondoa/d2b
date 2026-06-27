# Audio support for d2b VMs (virtio-snd + vhost-user-sound +
# PipeWire). Imported into the GUEST config by host.nix whenever a
# VM sets `d2b.vms.<name>.audio.enable = true`.
#
# Host-side wiring (the per-VM sidecar systemd service, the
# WirePlumber rule, the activation script that materialises per-VM
# state files) lives in modules/d2b/audio-host.nix and is imported
# at the host scope from modules/d2b/default.nix.
#
# Architecture
# ------------
# Cloud-hypervisor has no native virtio-snd. We use its
# `--generic-vhost-user` flag (see cloud-hypervisor/docs/generic-
# vhost-user.md) to attach a vhost-user backend. The backend is
# upstream `vhost-device-sound --backend pipewire`, which connects to
# the user's PipeWire daemon and appears in plasma-pa as a client
# named `d2b-<vm>` — giving the user a normal per-stream mute/
# volume UX through the Plasma mixer.
#
# The vhost-user protocol is 1:1 frontend<->backend, so one daemon
# process per VM that currently has audio. We use one per-VM system
# service (`d2b-<vm>-snd.service`, declared in audio-host.nix)
# and start it on demand when the VM has audio actively granted via
# `d2b audio …`.
#
# Boot-time enable: this module wires `microvm.extraArgsScript` to a
# tiny shell helper that reads /var/lib/d2b/<vm>/audio-state.json
# at VM start. If both mic and speaker are "off", the helper emits
# nothing — no virtio-snd device, the guest sees no soundcard. If at
# least one direction is "on", the helper:
#   1. Asks systemd to start `d2b-<vm>-snd.service` if not running.
#   2. Waits up to 5s for the vhost-user socket to appear under
#      /run/d2b/vms/<vm>/.
#   3. Echoes `--generic-vhost-user socket=...,virtio_id=25,
#      queue_sizes=[64,64,64,64]` on stdout, which microvm.nix's
#      runner template captures into `runtime_args` and appends to
#      the cloud-hypervisor command line.
#
# Split mic/speaker enforcement happens at the WirePlumber layer:
# audio-host.nix installs a rule that reads the same state file and
# null-routes the disabled direction's streams from the client.
# v1 keeps that mechanism simple; refinements are out of scope.
{ lib, pkgs, config, ... }:

let
  vmName = config.networking.hostName;
  d2bLib = import ../../lib.nix { inherit lib pkgs; };

  # The helper script invoked by microvm.nix's runner at VM start.
  # Output: either nothing (no audio device) or a single line of
  # additional cloud-hypervisor flags.
  #
  # Important: in the d2b framework, an `audio.enable = true` VM
  # is asserted to also have `autostart = false` (see the assertion
  # in audio-host.nix). That means the runner is always launched
  # interactively via `d2b up` running as the host's Wayland
  # user (`d2b.site.waylandUser`), never via microvm@<vm>.service.
  audioArgsScript = pkgs.writeShellScript "d2b-audio-args-${vmName}" ''
    set -eu
    # shellcheck source=/dev/null
    . ${d2bLib.d2bReadAudioState}
    _a_result=$(d2b_read_audio_state "${vmName}")
    mic=''${_a_result#mic=}; mic=''${mic% *}
    spk=''${_a_result#* speaker=}
    if [ "$mic" != "on" ] && [ "$spk" != "on" ]; then
      # Both directions off (or state unreadable/invalid) — no device attached.
      exit 0
    fi

    # Per-VM d2b-<vm>-snd.service puts the socket at this
    # path under RuntimeDirectory=d2b/vms/<vm>.
    sock="/run/d2b/vms/${vmName}/snd.sock"

    # Best-effort: ensure the sidecar's service is running. `d2b
    # up` (the CLI) is the canonical entry point and starts the sidecar
    # explicitly before launching CH. This block is a belt-and-
    # suspenders for direct `microvm -r <vm>` invocations and for
    # users who flip a state file by hand. We use the .service unit
    # (no socket activation — vhost-device-sound v0.2.0 has no
    # --socket-fd flag, see audio-host.nix for details).
    ${pkgs.systemd}/bin/systemctl reset-failed \
      "d2b-${vmName}-snd.service" >/dev/null 2>&1 || true
    ${pkgs.systemd}/bin/systemctl start \
      "d2b-${vmName}-snd.service" >/dev/null 2>&1 || true

    # Wait briefly for the listening socket to appear.
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      [ -S "$sock" ] && break
      ${pkgs.coreutils}/bin/sleep 0.5
    done
    if [ ! -S "$sock" ]; then
      echo "d2b-audio: sidecar socket $sock did not appear; skipping audio device" >&2
      exit 0
    fi

    # Cloud-hypervisor generic-vhost-user spec. virtio_id=25 is the
    # virtio device ID for "sound" per the virtio spec. queue_sizes
    # must be a 4-element list matching vhost-device-sound's
    # advertised queue count (ctrl + event + tx + rx). Available since
    # cloud-hypervisor v52.0 (the spectrum-ch package pins to v52).
    printf -- '--generic-vhost-user socket=%s,virtio_id=25,queue_sizes=[64,64,64,64]\n' "$sock"
  '';
in

{
  # In-guest audio user list — populated from the host-side
  # `d2b.vms.<name>.audio.users` (default `[ ssh.user ]`) via
  # the propagation pattern in host.nix. Declared as an option in
  # this module so the value resolves cleanly at guest-config eval.
  options.d2b.audio.users = lib.mkOption {
    type = lib.types.listOf lib.types.str;
    default = [ ];
    description = ''
      Guest-side usernames to add to the `audio` group so they can
      reach the virtio-snd device + PipeWire daemon without
      logind-active session privileges. Populated by host.nix from
      `d2b.vms.<name>.audio.users`.
    '';
  };

  config = {
  # Cloud-hypervisor is the only path: qemu+vhost-user-snd would
  # work too but the rest of d2b already targets CH. mkDefault so
  # graphics.nix / tpm.nix (which also pin cloud-hypervisor) don't
  # conflict.
  microvm.hypervisor = lib.mkDefault "cloud-hypervisor";

  # Dynamic per-boot --generic-vhost-user injection. See header
  # comment for the full lifecycle.
  microvm.extraArgsScript = "${audioArgsScript}";

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
  # The user list is declared host-side via
  # `d2b.vms.<name>.audio.users` and propagated into
  # `config.d2b.audio.users` here by host.nix. We add `audio` to
  # each listed user's extraGroups; users.users.<u>.extraGroups is a
  # list-merge type, so this composes with whatever the per-VM file
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
